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
        return none_or_error(arg_name, allow_none);
    }

    require_time_instance(value, arg_name)?;
    let fields = TimeFields::extract(value)?;
    require_naive_timezone(value, arg_name)?;
    fields.into_naive_time(value, arg_name)
}

/// The clock fields read from a Python `datetime.time`.
struct TimeFields {
    /// Hour of day, 0–23.
    hour: u32,
    /// Minute of hour, 0–59.
    minute: u32,
    /// Second of minute, 0–59.
    second: u32,
    /// Microsecond of second, 0–999_999.
    microsecond: u32,
}

impl TimeFields {
    /// Read the four clock attributes from `value`.
    ///
    /// Attributes are read before the timezone is inspected so a malformed
    /// value reports its field error first, as it always has.
    fn extract(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            hour: value.getattr("hour")?.extract()?,
            minute: value.getattr("minute")?.extract()?,
            second: value.getattr("second")?.extract()?,
            microsecond: value.getattr("microsecond")?.extract()?,
        })
    }

    /// Build the [`NaiveTime`], rejecting out-of-range field combinations.
    fn into_naive_time(
        self,
        value: &Bound<'_, PyAny>,
        arg_name: &str,
    ) -> PyResult<Option<NaiveTime>> {
        NaiveTime::from_hms_micro_opt(self.hour, self.minute, self.second, self.microsecond)
            .ok_or_else(|| {
                PyTypeError::new_err(format!(
                    "invalid {arg_name} value of type {}",
                    fq_py_type(value)
                ))
            })
            .map(Some)
    }
}

/// Return `Ok(None)` for an allowed `None`, otherwise report the wrong type.
fn none_or_error(arg_name: &str, allow_none: bool) -> PyResult<Option<NaiveTime>> {
    if allow_none {
        return Ok(None);
    }

    Err(PyTypeError::new_err(format!(
        "{arg_name} must be datetime.time or None"
    )))
}

/// Reject objects that are not a `datetime.time`.
fn require_time_instance(value: &Bound<'_, PyAny>, arg_name: &str) -> PyResult<()> {
    let py = value.py();
    let time_type = py.import("datetime")?.getattr("time")?;
    if value.is_instance(&time_type)? {
        return Ok(());
    }

    Err(PyTypeError::new_err(format!(
        "{arg_name} must be datetime.time, got {}",
        fq_py_type(value),
    )))
}

/// Reject timezone-aware times, which a `NaiveTime` cannot represent.
fn require_naive_timezone(value: &Bound<'_, PyAny>, arg_name: &str) -> PyResult<()> {
    if value.getattr("tzinfo")?.is_none() {
        return Ok(());
    }

    Err(PyValueError::new_err(format!(
        "{arg_name} must be timezone-naive"
    )))
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
