//! Validation helpers for file-handler constructor parameters.
//!
//! Keeping the guards here lets the main handler module re-export the same
//! internal helpers while staying below the project file-size limit.

use pyo3::PyResult;

pub(crate) const CAPACITY_ZERO_MSG: &str = "capacity must be greater than zero";
pub(crate) const FLUSH_INTERVAL_ZERO_MSG: &str = "flush_interval must be greater than zero";

pub(crate) fn validate_capacity_nonzero(capacity: usize) -> Result<(), &'static str> {
    if capacity == 0 {
        return Err(CAPACITY_ZERO_MSG);
    }
    Ok(())
}

/// Validate a possibly negative flush interval from FFI or other input-boundary
/// sources and return a usable `usize` on success.
pub(crate) fn validate_flush_interval_value(flush_interval: isize) -> Result<usize, &'static str> {
    if flush_interval <= 0 {
        return Err(FLUSH_INTERVAL_ZERO_MSG);
    }
    Ok(flush_interval as usize)
}

/// Validate a Rust-side non-negative flush interval that only needs a zero
/// guard, mirroring `validate_capacity_nonzero` for internal callers.
pub(crate) fn validate_flush_interval_nonzero(flush_interval: usize) -> Result<(), &'static str> {
    if flush_interval == 0 {
        return Err(FLUSH_INTERVAL_ZERO_MSG);
    }
    Ok(())
}

pub(crate) fn validate_params(capacity: usize, flush_interval: isize) -> PyResult<usize> {
    use pyo3::exceptions::PyValueError;

    validate_capacity_nonzero(capacity).map_err(PyValueError::new_err)?;
    validate_flush_interval_value(flush_interval).map_err(PyValueError::new_err)
}
