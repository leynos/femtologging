//! Conversion of Python `datetime.time` values for timed rotation.
//!
//! The timed-rotation options and their builder both accept an optional local
//! trigger, so the validation and error messages live here rather than in the
//! `PyO3` class that happens to call them first.

use chrono::NaiveTime;
use pyo3::{
    Bound,
    exceptions::{PyTypeError, PyValueError},
    prelude::*,
};

use crate::python::fq_py_type;

/// Extract a `NaiveTime` from a Python `datetime.time` object.
///
/// # Parameters
///
/// - `value`: The Python object to extract from
/// - `arg_name`: The name of the argument (for error messages)
/// - `allow_none`: If `true`, `None` values return `Ok(None)`; if `false`, they
///   raise an error
///
/// # Errors
///
/// Returns an error if:
/// - `value` is `None` and `allow_none` is `false`
/// - The time object has a non-None `tzinfo` (timezone-aware)
/// - The extracted hour/minute/second/microsecond values are invalid
pub(crate) fn extract_naive_time_from_py_time(
    value: &Bound<'_, PyAny>,
    arg_name: &str,
    allow_none: bool,
) -> PyResult<Option<NaiveTime>> {
    if value.is_none() {
        if allow_none {
            return Ok(None);
        }

        return Err(PyTypeError::new_err(format!(
            "{arg_name} must be datetime.time or None"
        )));
    }

    let py = value.py();
    let time_type = py.import("datetime")?.getattr("time")?;
    if !value.is_instance(&time_type)? {
        return Err(PyTypeError::new_err(format!(
            "{arg_name} must be datetime.time, got {}",
            fq_py_type(value),
        )));
    }

    let hour: u32 = value.getattr("hour")?.extract()?;
    let minute: u32 = value.getattr("minute")?.extract()?;
    let second: u32 = value.getattr("second")?.extract()?;
    let microsecond: u32 = value.getattr("microsecond")?.extract()?;
    let tzinfo = value.getattr("tzinfo")?;

    if !tzinfo.is_none() {
        return Err(PyValueError::new_err(format!(
            "{arg_name} must be timezone-naive"
        )));
    }

    NaiveTime::from_hms_micro_opt(hour, minute, second, microsecond)
        .ok_or_else(|| {
            PyTypeError::new_err(format!(
                "invalid {arg_name} value of type {}",
                fq_py_type(value)
            ))
        })
        .map(Some)
}

/// Converts a Python datetime.time value using the common validation helper.
pub(super) fn extract_naive_time(value: Bound<'_, PyAny>) -> PyResult<NaiveTime> {
    // Local convenience wrapper for the common helper; this retains the
    // existing `at_time`-specific error messages.
    match extract_naive_time_from_py_time(&value, "at_time", false)? {
        Some(time) => Ok(time),
        // `allow_none` is false, so the helper rejects None before reaching
        // this arm. A defensive error keeps the contract visible.
        None => Err(PyTypeError::new_err(
            "at_time must be datetime.time or None",
        )),
    }
}
