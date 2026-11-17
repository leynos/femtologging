//! Python helper functions for FemtoLogger.
//!
//! This module contains utility functions for Python integration, including
//! exception capture logic.

#[cfg(feature = "python")]
use pyo3::prelude::*;
#[cfg(feature = "python")]
use pyo3::types::PyBool;

/// Return `exc_info` when it requests capture, after validating its shape.
///
/// Returning the reference rather than a boolean keeps the decision and the
/// validated value together, so the deferred capture hook cannot mistake an
/// unvalidated argument for a validated one. Invalid `exc_info` values raise
/// `TypeError` here, before any filter sees the record.
#[cfg(feature = "python")]
pub(super) fn validated_exc_info<'py, 'a>(
    py: Python<'py>,
    exc_info: Option<&'a Bound<'py, PyAny>>,
) -> PyResult<Option<&'a Bound<'py, PyAny>>> {
    let Some(exc) = exc_info else {
        return Ok(None);
    };
    if !should_capture_exc_info(exc)? {
        return Ok(None);
    }
    crate::traceback_capture::validate_exc_info(py, exc)?;
    Ok(Some(exc))
}

/// Attach bounded exception and stack payloads to `record`.
///
/// This is the deferred-capture hook used by the producer's two-phase filter
/// path. `exc_info` has already been filtered and validated by
/// [`validated_exc_info`], so the only failures left here are interpreter-level
/// ones; those are dropped rather than propagated, because logging must not
/// fail on the way out of a failure.
#[cfg(feature = "python")]
pub(super) fn attach_captured_payloads(
    py: Python<'_>,
    exc_info: Option<&Bound<'_, PyAny>>,
    stack_info: Option<bool>,
    record: &mut crate::log_record::FemtoLogRecord,
) {
    if let Some(exc) = exc_info
        && let Ok(Some(payload)) = crate::traceback_capture::capture_exception(py, exc)
    {
        record.set_exception_payload(payload);
    }

    if stack_info.unwrap_or(false)
        && let Ok(payload) = crate::traceback_capture::capture_stack(py)
    {
        record.set_stack_payload(payload);
    }
}

/// Determine whether exc_info should trigger exception capture.
///
/// Returns `true` for any non-False, non-None value (including exception
/// instances and 3-tuples). The actual type validation happens in
/// [`capture_exception`], which will raise `TypeError` for invalid types.
///
/// Returns `false` for `False` or `None` values—these explicitly disable
/// exception capture.
#[cfg(feature = "python")]
pub fn should_capture_exc_info(exc_info: &Bound<'_, PyAny>) -> PyResult<bool> {
    // Handle boolean False explicitly
    if let Ok(b) = exc_info.cast::<PyBool>() {
        return Ok(b.is_true());
    }
    // None means no capture
    if exc_info.is_none() {
        return Ok(false);
    }
    // Any other value (exception instance, tuple, or invalid type) triggers
    // capture attempt. Invalid types will fail in capture_exception.
    Ok(true)
}
