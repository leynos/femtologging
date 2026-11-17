//! Helpers for classifying real Python exception objects.

use pyo3::prelude::*;

/// Return `true` when `value` is a real Python exception instance.
pub(super) fn is_exception(py: Python<'_>, value: &Bound<'_, PyAny>) -> bool {
    let Ok(builtins) = py.import("builtins") else {
        return false;
    };
    let Ok(base_exception) = builtins.getattr("BaseException") else {
        return false;
    };
    value.is_instance(&base_exception).unwrap_or(false)
}

/// Return `true` when `exception` is an exception group.
pub(super) fn is_exception_group(py: Python<'_>, exception: &Bound<'_, PyAny>) -> bool {
    let Ok(builtins) = py.import("builtins") else {
        return false;
    };
    let Ok(group_type) = builtins.getattr("BaseExceptionGroup") else {
        // Python older than 3.11 has no exception groups.
        return false;
    };
    exception.is_instance(&group_type).unwrap_or(false)
}
