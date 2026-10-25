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

mod max_level;

use std::borrow::Cow;
use std::sync::{Arc, OnceLock, atomic::AtomicU8};

use log::{Metadata, Record};
use pyo3::prelude::*;

use crate::level::FemtoLevel;
use crate::log_record::RecordMetadata;
use crate::logger::DEFAULT_LOGGER_LEVEL;
use crate::manager;

/// Adapter implementing the Rust `log::Log` trait.
///
/// The adapter resolves a femtologging logger based on each record's target,
/// converts the record to a [`FemtoLogRecord`], and dispatches it through the
/// logger's handler queue.
pub struct FemtoLogAdapter;

/// Maps facade levels without changing the ordering used by logger enablement checks.
fn map_log_level(level: log::Level) -> FemtoLevel {
    match level {
        log::Level::Trace => FemtoLevel::Trace,
        log::Level::Debug => FemtoLevel::Debug,
        log::Level::Info => FemtoLevel::Info,
        log::Level::Warn => FemtoLevel::Warn,
        log::Level::Error => FemtoLevel::Error,
    }
}

/// Maps internal levels back to the facade for callers that observe forwarded records.
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

/// Normalises the global facade target before it selects a Python-owned logger.
fn normalize_target(target: &str) -> Cow<'_, str> {
    if target.contains("::") {
        Cow::Owned(target.replace("::", "."))
    } else {
        Cow::Borrowed(target)
    }
}

/// Distinguishes a missing Python logger from an error that must not be swallowed.
fn is_unknown_logger_error(py: Python<'_>, err: &PyErr) -> bool {
    err.is_instance_of::<pyo3::exceptions::PyKeyError>(py)
}

/// Recognises invalid names so facade records can be rejected without masking Python faults.
fn is_invalid_logger_error(py: Python<'_>, err: &PyErr) -> bool {
    err.is_instance_of::<pyo3::exceptions::PyValueError>(py)
}

/// Classify a logger resolution error for diagnostic logging.
fn classify_logger_error(py: Python<'_>, err: &PyErr) -> &'static str {
    if is_unknown_logger_error(py, err) {
        "unknown logger target"
    } else if is_invalid_logger_error(py, err) {
        "invalid logger target"
    } else {
        "unexpected error resolving logger"
    }
}

/// Resolves a facade target while attached to the GIL, returning an owned logger for later use.
fn resolve_logger<'py>(py: Python<'py>, target: &str) -> Option<(String, Py<crate::FemtoLogger>)> {
    let normalized = normalize_target(target);
    match manager::get_logger(py, normalized.as_ref()) {
        Ok(logger) => Some((normalized.into_owned(), logger)),
        Err(err) => {
            let reason = classify_logger_error(py, &err);
            log::warn!(
                target: "femtologging.log_compat",
                "femtologging: {reason} {:?} (normalized {:?}); falling back to root: {}",
                target,
                normalized.as_ref(),
                err
            );
            let logger = manager::get_logger(py, "root").ok()?;
            Some(("root".to_string(), logger))
        }
    }
}

/// Applies the facade's global maximum before crossing into Python logger resolution.
fn is_enabled_by_global_max(level: log::Level) -> bool {
    log::max_level() >= level.to_level_filter()
}

/// Resolve a normalized target's threshold, using root or the default as fallback.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(effective_logger_level("unknown.target"), DEFAULT_LOGGER_LEVEL);
/// ```
fn effective_logger_level(target: &str) -> FemtoLevel {
    let normalized = normalize_target(target);
    let logger_name = if manager::is_valid_logger_name(normalized.as_ref()) {
        normalized.as_ref()
    } else {
        "root"
    };
    manager::registered_logger_level(logger_name).unwrap_or(DEFAULT_LOGGER_LEVEL)
}

/// Compare a Rust record level with the target logger's femtologging threshold.
///
/// # Examples
///
/// ```ignore
/// assert!(!is_enabled_by_logger_level(log::Level::Debug, FemtoLevel::Warn));
/// assert!(is_enabled_by_logger_level(log::Level::Warn, FemtoLevel::Warn));
/// ```
fn is_enabled_by_logger_level(level: log::Level, threshold: FemtoLevel) -> bool {
    u8::from(FemtoLevel::from(level)) >= u8::from(threshold)
}

impl log::Log for FemtoLogAdapter {
    /// Return whether the target's current level threshold accepts this record.
    ///
    /// Target names are normalized from Rust's `module::path` form to dotted
    /// femtologging names. Unknown valid targets use the default logger level;
    /// invalid names use the registered root logger level when available, then
    /// the default. This check does not inherit parent levels or consult
    /// filters and handlers, and it never creates a logger or accesses Python.
    /// A `true` result means the level threshold accepts the record; filters
    /// and handlers may still drop it during dispatch.
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        is_enabled_by_global_max(metadata.level())
            && is_enabled_by_logger_level(
                metadata.level(),
                effective_logger_level(metadata.target()),
            )
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
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

            let _ =
                logger
                    .borrow(py)
                    .log_with_metadata(level, &record.args().to_string(), metadata);
        });
    }

    fn flush(&self) {
        Python::attach(|py| {
            manager::flush_all_handlers(py);
        });
    }
}

/// Provides the single global adapter required by the `log` facade installation contract.
static FEMTO_LOG_ADAPTER: FemtoLogAdapter = FemtoLogAdapter;

/// Records the one-time installation result so competing initialisers observe a stable outcome.
static INSTALL_RESULT: OnceLock<bool> = OnceLock::new();

/// Recompute the facade's broadest accepted level after registry changes.
///
/// The process-wide maximum includes the default level for unknown targets,
/// so it can skip macro arguments only when every registered target and the
/// default would reject that level. It is refreshed without taking the
/// manager lock; the index's lock ordering is always manager then index.
pub(crate) fn refresh_global_max_level() {
    max_level::refresh_global_max_level();
}

/// Update a logger threshold while keeping the global prefilter conservative.
pub(crate) fn update_logger_level(level: &Arc<AtomicU8>, new_level: FemtoLevel) {
    max_level::update_logger_level(level, new_level);
}

/// Install femtologging as the global Rust logger.
///
/// Returns `true` on success. When a different global logger is already set,
/// installation fails and `false` is returned. Subsequent calls return the
/// cached outcome.
pub fn install_global_logger() -> bool {
    let installed = *INSTALL_RESULT.get_or_init(|| log::set_logger(&FEMTO_LOG_ADAPTER).is_ok());
    if installed {
        refresh_global_max_level();
    }
    installed
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

/// Probe the public `log_enabled!` macro from Python behavioural tests.
#[cfg(feature = "python")]
#[pyfunction]
#[pyo3(name = "_rust_log_enabled")]
pub(crate) fn rust_log_enabled(level: FemtoLevel, target: &str) -> bool {
    let mapped = map_femto_to_log_level(level);
    log::log_enabled!(target: target, mapped)
}

/// Return the facade maximum for Python behavioural tests.
#[cfg(feature = "python")]
#[pyfunction]
#[pyo3(name = "_rust_log_max_level")]
pub(crate) fn rust_log_max_level() -> String {
    log::max_level().to_string()
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

#[cfg(test)]
#[path = "log_compat_context_tests.rs"]
mod context_tests;

#[cfg(test)]
#[path = "log_compat/tests.rs"]
mod tests;
