//! Python helper functions for FemtoLogger.
//!
//! This module contains utility functions for Python integration, including
//! exception capture logic.

#[cfg(feature = "python")]
use pyo3::prelude::*;
#[cfg(feature = "python")]
use pyo3::types::PyBool;

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
    if let Ok(b) = exc_info.downcast::<PyBool>() {
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
