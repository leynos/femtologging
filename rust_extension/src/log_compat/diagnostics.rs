//! Keep the Rust log bridge's own diagnostics visible without recursive dispatch.
//!
//! The adapter is the global Rust logger, so anything it reports about its own
//! work would be routed straight back into itself. This module holds the guard
//! that drops those records, the out-of-band mirror that keeps internal
//! warnings visible, and the rate-limited diagnostic a target refused by the
//! bounded registry produces before the bridge falls back to root.

use std::sync::LazyLock;

use log::Record;
use pyo3::prelude::*;

use crate::manager;
use crate::rate_limited_warner::{DEFAULT_WARN_INTERVAL, RateLimitedWarner};

use super::normalize_target;

/// Return whether a target belongs to femtologging itself.
///
/// The adapter is the global Rust logger, so a record it logs about its own
/// work would be routed straight back into the adapter. The lookup-failure
/// diagnostic is the concrete case: it reports a target the registry refused,
/// which means the diagnostic describes a high-cardinality stream — and an
/// unguarded warning would add one more record per refusal to it. The tracing
/// bridge ignores its own targets for the same reason.
///
/// The `femtologging` family covers logger names this crate uses for itself;
/// the crate's own `log` targets cover module paths such as
/// `_femtologging_rs::logger::dispatcher::shard::worker`. The check compares
/// raw targets so the common module-path case allocates nothing.
pub(super) fn should_ignore_target(target: &str) -> bool {
    let is_in_family = |root: &str| {
        target
            .strip_prefix(root)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('.') || rest.starts_with("::"))
    };
    is_in_family("femtologging") || is_in_family(env!("CARGO_CRATE_NAME"))
}

/// Mirror an ignored crate warning through an out-of-band writer.
///
/// For example, dispatch-capacity warnings are mirrored to stderr instead of
/// being forwarded back into the adapter that could not accept their record.
pub(super) fn mirror_ignored_warning(
    record: &Record<'_>,
    write_warning: impl FnOnce(&std::fmt::Arguments<'_>),
) {
    if should_ignore_target(record.target()) && record.level() == log::Level::Warn {
        write_warning(record.args());
    }
}

/// Identify the `KeyError` used when a requested logger name is absent.
///
/// For example, a lookup failure represented as `PyKeyError` returns `true`.
fn is_unknown_logger_error(py: Python<'_>, err: &PyErr) -> bool {
    err.is_instance_of::<pyo3::exceptions::PyKeyError>(py)
}

/// Identify validation refusals represented as `ValueError`.
///
/// For example, an empty logger name returns `true` after validation refuses it.
fn is_invalid_logger_error(py: Python<'_>, err: &PyErr) -> bool {
    err.is_instance_of::<pyo3::exceptions::PyValueError>(py)
}

/// True when the registry refused the target because it is at
/// [`MAX_LOGGERS`](crate::manager::MAX_LOGGERS).
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

/// Borrowed inputs for a refused-target diagnostic.
pub(super) struct LookupFailureDiagnostic<'a> {
    /// Classification of the refused lookup.
    pub(super) reason: &'a str,
    /// Caller-provided target that could not be resolved.
    pub(super) target: &'a str,
    /// Candidate name passed to the registry after bridge normalization.
    pub(super) normalized: &'a str,
    /// Registry error text shown with the fallback diagnostic.
    pub(super) detail: &'a str,
    /// Records this warning accounts for, including the one that triggered it.
    pub(super) count: u64,
}

/// Build the stderr diagnostic for a target the registry refused.
///
/// Split out from the warner so the size bound is directly testable. Both names
/// are caller-controlled and neither has necessarily passed name validation —
/// this runs precisely when one did not — so both are quoted through
/// [`manager::describe_name`], which escapes control characters and stops at a
/// fixed prefix. Quoting them verbatim would let one warning write an unbounded
/// line, and an unescaped one could forge a record boundary in the same
/// line-oriented output the name rules exist to protect.
pub(super) fn lookup_failure_message(diagnostic: LookupFailureDiagnostic<'_>) -> String {
    let LookupFailureDiagnostic {
        reason,
        target,
        normalized,
        detail,
        count,
    } = diagnostic;
    format!(
        "femtologging: {reason} {} (normalized {}); falling back to root: {detail}; \
         {count} record(s) affected since the last warning",
        manager::describe_name(target),
        manager::describe_name(normalized),
    )
}

/// Rate-limit and report a bridge target that the registry refused.
///
/// For example, repeated records to a full-registry target share one stderr
/// warning per configured interval instead of writing once per record.
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
            lookup_failure_message(LookupFailureDiagnostic {
                reason,
                target,
                normalized,
                detail: &detail,
                count,
            })
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

/// Resolve a normalized target, warning and falling back to root on refusal.
///
/// For example, a valid `my::service` target resolves to `my.service`; a name
/// refused by the bounded registry resolves to the root logger.
pub(super) fn resolve_logger<'py>(
    py: Python<'py>,
    target: &str,
) -> Option<(String, Py<crate::FemtoLogger>)> {
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
