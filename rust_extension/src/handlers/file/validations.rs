//! Validation helpers for file-handler constructor parameters.
//!
//! Keeping the guards here lets the main handler module re-export the same
//! internal helpers while staying below the project file-size limit.

use pyo3::PyResult;

/// Error message returned when a channel or batch capacity is zero.
pub(crate) const CAPACITY_ZERO_MSG: &str = "capacity must be greater than zero";
/// Error message returned when a flush interval is zero or negative.
pub(crate) const FLUSH_INTERVAL_ZERO_MSG: &str = "flush_interval must be greater than zero";

/// Reject zero-sized capacities at the Rust API boundary.
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

/// Validate constructor parameters exposed through Python bindings.
pub(crate) fn validate_params(capacity: usize, flush_interval: isize) -> PyResult<usize> {
    use pyo3::exceptions::PyValueError;

    validate_capacity_nonzero(capacity).map_err(PyValueError::new_err)?;
    validate_flush_interval_value(flush_interval).map_err(PyValueError::new_err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_capacity_nonzero_handles_boundaries() {
        assert_eq!(validate_capacity_nonzero(0), Err(CAPACITY_ZERO_MSG));
        assert_eq!(validate_capacity_nonzero(1), Ok(()));
    }

    #[test]
    fn validate_flush_interval_value_handles_boundaries() {
        assert_eq!(
            validate_flush_interval_value(-1),
            Err(FLUSH_INTERVAL_ZERO_MSG)
        );
        assert_eq!(
            validate_flush_interval_value(0),
            Err(FLUSH_INTERVAL_ZERO_MSG)
        );
        assert_eq!(validate_flush_interval_value(1), Ok(1));
    }

    #[test]
    fn validate_flush_interval_nonzero_handles_boundaries() {
        assert_eq!(
            validate_flush_interval_nonzero(0),
            Err(FLUSH_INTERVAL_ZERO_MSG)
        );
        assert_eq!(validate_flush_interval_nonzero(2), Ok(()));
    }

    #[test]
    fn validate_params_returns_python_errors_for_invalid_values() {
        let capacity_err = validate_params(0, 1).expect_err("zero capacity should fail");
        assert!(capacity_err.to_string().contains(CAPACITY_ZERO_MSG));

        let flush_err = validate_params(1, 0).expect_err("zero flush interval should fail");
        assert!(flush_err.to_string().contains(FLUSH_INTERVAL_ZERO_MSG));
    }

    #[test]
    fn validate_params_returns_flush_interval_on_success() {
        assert_eq!(validate_params(2, 3).expect("valid params should pass"), 3);
    }
}
