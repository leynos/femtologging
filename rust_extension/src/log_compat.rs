//! Compatibility bridge for the Rust `log` crate.
//!
//! This module provides `FemtoLogAdapter`, an implementation of
//! `log::Log` that forwards Rust-side log records into femtologging's
//! asynchronous handler pipeline. The bridge is enabled explicitly from
//! Python via `setup_rust_logging()`, which installs the adapter as the
//! global Rust logger. Each bridged record captures the Rust scoped context
//! active on the emitting OS thread; explicit record metadata takes precedence
//! for duplicate keys. Python task-local context is not transferred to this
//! Rust bridge.

use std::borrow::Cow;
use std::sync::{LazyLock, OnceLock};

use log::{Metadata, Record};
use pyo3::prelude::*;

use crate::level::FemtoLevel;
use crate::log_record::RecordMetadata;
use crate::manager;
use crate::rate_limited_warner::{DEFAULT_WARN_INTERVAL, RateLimitedWarner};

/// Adapter implementing the Rust `log::Log` trait.
///
/// The adapter resolves a femtologging logger based on each record's target,
/// converts the record to a [`FemtoLogRecord`], and dispatches it through the
/// logger's handler queue.
pub struct FemtoLogAdapter;

fn map_log_level(level: log::Level) -> FemtoLevel {
    match level {
        log::Level::Trace => FemtoLevel::Trace,
        log::Level::Debug => FemtoLevel::Debug,
        log::Level::Info => FemtoLevel::Info,
        log::Level::Warn => FemtoLevel::Warn,
        log::Level::Error => FemtoLevel::Error,
    }
}

fn map_femto_to_log_level(level: FemtoLevel) -> log::Level {
    match level {
        FemtoLevel::Trace => log::Level::Trace,
        FemtoLevel::Debug => log::Level::Debug,
        FemtoLevel::Info => log::Level::Info,
        FemtoLevel::Warn => log::Level::Warn,
        FemtoLevel::Error | FemtoLevel::Critical => log::Level::Error,
    }
}

impl From<log::Level> for FemtoLevel {
    fn from(level: log::Level) -> Self {
        map_log_level(level)
    }
}

fn normalize_target(target: &str) -> Cow<'_, str> {
    if target.contains("::") {
        Cow::Owned(target.replace("::", "."))
    } else {
        Cow::Borrowed(target)
    }
}

/// The `log` target prefix this crate's own records carry.
///
/// `log::warn!` with no explicit target records the module path, whose first
/// segment is the crate name. Naming the crate through `env!` rather than
/// repeating the manifest's `[lib] name` keeps the two from drifting apart.
const OWN_TARGET_PREFIX: &str = concat!(env!("CARGO_CRATE_NAME"), ".");

/// Return whether a target belongs to femtologging itself.
///
/// The adapter is the global Rust logger, so a record it logs about its own
/// work would be routed straight back into the adapter. The lookup-failure
/// diagnostic is the concrete case: it reports a target the registry refused,
/// which means the diagnostic describes a high-cardinality stream — and an
/// unguarded warning would add one more record per refusal to it. The tracing
/// bridge ignores its own targets for the same reason.
///
/// Two families are covered. The `femtologging` names are the ones this crate
/// uses when it names a logger for itself. The crate's own `log` targets are
/// the wider family and the more important one here: every `warn!` in this
/// crate records a module path such as
/// `_femtologging_rs::logger::dispatcher::shard::worker`, and those warnings
/// fire exactly when the process is short of a resource. Forwarding one back
/// through the adapter would add a record per warning to the stream being
/// diagnosed — which is a loop the bridge itself has no way to stop.
fn should_ignore_target(target: &str) -> bool {
    let normalized = normalize_target(target);
    normalized == "femtologging"
        || normalized.starts_with("femtologging.")
        || normalized == env!("CARGO_CRATE_NAME")
        || normalized.starts_with(OWN_TARGET_PREFIX)
}

fn is_unknown_logger_error(py: Python<'_>, err: &PyErr) -> bool {
    err.is_instance_of::<pyo3::exceptions::PyKeyError>(py)
}

fn is_invalid_logger_error(py: Python<'_>, err: &PyErr) -> bool {
    err.is_instance_of::<pyo3::exceptions::PyValueError>(py)
}

/// True when the registry refused the target because it is at [`MAX_LOGGERS`].
///
/// The refusal shares `RuntimeError` with genuine faults, so it is recognized
/// by the message the registry builds rather than by its class.
fn is_registry_full_error(py: Python<'_>, err: &PyErr) -> bool {
    err.is_instance_of::<pyo3::exceptions::PyRuntimeError>(py)
        && err.value(py).str().is_ok_and(|text| {
            text.to_string_lossy()
                .contains(manager::REGISTRY_FULL_MARKER)
        })
}

/// Warn at most once per [`DEFAULT_WARN_INTERVAL`] for repeated bridge failures.
///
/// A high-cardinality target keeps failing for every record it emits, so an
/// unthrottled warning would make the diagnostic itself a record amplifier.
static LOOKUP_WARNER: LazyLock<RateLimitedWarner> =
    LazyLock::new(|| RateLimitedWarner::new(DEFAULT_WARN_INTERVAL));

/// Build the stderr diagnostic for a target the registry refused.
///
/// Split out from the warner so the size bound is directly testable. Both names
/// are caller-controlled and neither has necessarily passed name validation —
/// this runs precisely when one did not — so both are quoted through
/// [`manager::describe_name`], which escapes control characters and stops at a
/// fixed prefix. Quoting them verbatim would let one warning write an unbounded
/// line, and an unescaped one could forge a record boundary in the same
/// line-oriented output the name rules exist to protect.
fn lookup_failure_message(
    reason: &str,
    target: &str,
    normalized: &str,
    detail: &str,
    count: u64,
) -> String {
    format!(
        "femtologging: {reason} {} (normalized {}); falling back to root: {detail}; \
         {count} further record(s) affected",
        manager::describe_name(target),
        manager::describe_name(normalized),
    )
}

fn warn_lookup_failure(py: Python<'_>, err: &PyErr, target: &str, normalized: &str) {
    LOOKUP_WARNER.record_drop();
    // Both the classification and the message are built inside the closure
    // because the warner discards most calls to this function, and a refused
    // target stream makes those discarded calls the common case. Building the
    // diagnostic first would allocate a `String` per refusal — on the very
    // path that exists because the process is already under pressure.
    LOOKUP_WARNER.warn_if_due(|count| {
        let reason = classify_logger_error(py, err);
        let detail = err.to_string();
        // Stderr, not `log::warn!`. This diagnostic's whole subject is a target
        // the bridge could not resolve, and the adapter is the global Rust
        // logger: routing the warning through it would send a record whose
        // target is `femtologging.log_compat` straight back into the adapter.
        // `should_ignore_target` discards exactly that target, so a `log::warn!`
        // here is written and immediately dropped — leaving a registry at its
        // cap with no visible diagnostic at all on the bridge path. Writing to
        // stderr bypasses the logger being diagnosed.
        eprintln!(
            "{}",
            lookup_failure_message(reason, target, normalized, &detail, count)
        );
    });
}

/// Classify a logger resolution error for diagnostic logging.
fn classify_logger_error(py: Python<'_>, err: &PyErr) -> &'static str {
    if is_unknown_logger_error(py, err) {
        "unknown logger target"
    } else if is_invalid_logger_error(py, err) {
        "invalid logger target"
    } else if is_registry_full_error(py, err) {
        "logger registry is full"
    } else {
        "unexpected error resolving logger"
    }
}

fn resolve_logger<'py>(py: Python<'py>, target: &str) -> Option<(String, Py<crate::FemtoLogger>)> {
    let normalized = normalize_target(target);
    match manager::get_logger(py, normalized.as_ref()) {
        Ok(logger) => Some((normalized.into_owned(), logger)),
        Err(err) => {
            warn_lookup_failure(py, &err, target, normalized.as_ref());
            // Root is always resolvable: it is exempt from the registry cap, so
            // this fallback cannot itself be refused.
            let logger = manager::get_logger(py, "root").ok()?;
            Some(("root".to_string(), logger))
        }
    }
}

fn is_enabled_by_global_max(level: log::Level) -> bool {
    log::max_level() >= level.to_level_filter()
}

impl log::Log for FemtoLogAdapter {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        is_enabled_by_global_max(metadata.level())
    }

    fn log(&self, record: &Record<'_>) {
        if !is_enabled_by_global_max(record.level()) {
            return;
        }
        if should_ignore_target(record.target()) {
            return;
        }

        Python::attach(|py| {
            let Some((_, logger)) = resolve_logger(py, record.target()) else {
                return;
            };

            let level = FemtoLevel::from(record.level());
            if !logger.borrow(py).is_enabled_for(level) {
                return;
            }

            let metadata = RecordMetadata {
                module_path: record.module_path().unwrap_or_default().to_string(),
                filename: record.file().unwrap_or_default().to_string(),
                line_number: record.line().unwrap_or(0),
                ..Default::default()
            };

            let _ = logger.borrow(py).log_with_metadata(
                level,
                &record.args().to_string(),
                metadata,
            );
        });
    }

    fn flush(&self) {
        Python::attach(|py| {
            manager::flush_all_handlers(py);
        });
    }
}

static FEMTO_LOG_ADAPTER: FemtoLogAdapter = FemtoLogAdapter;
static INSTALL_RESULT: OnceLock<bool> = OnceLock::new();

/// Install femtologging as the global Rust logger.
///
/// Returns `true` on success. When a different global logger is already set,
/// installation fails and `false` is returned. Subsequent calls return the
/// cached outcome.
pub(crate) fn install_global_logger() -> bool {
    *INSTALL_RESULT.get_or_init(|| {
        if log::set_logger(&FEMTO_LOG_ADAPTER).is_err() {
            return false;
        }
        log::set_max_level(log::LevelFilter::Trace);
        true
    })
}

/// Python-facing entrypoint for enabling the `log` crate bridge.
///
/// Installs the adapter as the global Rust logger. The operation is
/// idempotent: repeated calls after a successful install are no-ops.
#[cfg(feature = "python")]
#[pyfunction]
pub(crate) fn setup_rust_logging() -> PyResult<()> {
    if install_global_logger() {
        Ok(())
    } else {
        Err(pyo3::exceptions::PyRuntimeError::new_err(
            "global Rust logger is already set; femtologging cannot install the log bridge",
        ))
    }
}

/// Emit a Rust log record via the `log` crate.
///
/// This is an internal helper used by the Python behavioural tests to validate
/// the bridge. `CRITICAL` maps to `ERROR` because the `log` crate has no
/// critical level.
#[cfg(feature = "python")]
#[pyfunction]
#[pyo3(name = "_emit_rust_log")]
pub(crate) fn emit_rust_log(level: FemtoLevel, message: &str, target: Option<&str>) {
    let mapped = map_femto_to_log_level(level);

    if let Some(target) = target {
        log::log!(target: target, mapped, "{}", message);
    } else {
        log::log!(mapped, "{}", message);
    }
}

/// Install a dummy global Rust logger for behavioural tests.
///
/// This helper is intended for subprocess-based test scenarios that need to
/// verify `setup_rust_logging()` fails when a different global logger has
/// already been configured.
#[cfg(feature = "python")]
#[pyfunction]
#[pyo3(name = "_install_test_global_rust_logger")]
pub(crate) fn install_test_global_rust_logger() -> PyResult<()> {
    struct TestLogger;

    impl log::Log for TestLogger {
        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }

        fn log(&self, _record: &Record<'_>) {}

        fn flush(&self) {}
    }

    static TEST_LOGGER: TestLogger = TestLogger;
    log::set_logger(&TEST_LOGGER).map_err(|_| {
        pyo3::exceptions::PyRuntimeError::new_err("global Rust logger is already set")
    })?;
    log::set_max_level(log::LevelFilter::Trace);
    Ok(())
}

// Bridge stress tests live in a sibling module to respect
// the 400-line limit.
#[cfg(test)]
#[path = "log_compat_context_tests.rs"]
mod context_tests;

#[cfg(test)]
#[path = "log_compat_tests.rs"]
mod tests;
