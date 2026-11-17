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
    if is_allowed_none(value, arg_name, allow_none)? {
        return Ok(None);
    }

    let time_type = python_time_type(value)?;
    ensure_is_time(value, &time_type, arg_name)?;
    let components = read_time_components(value)?;
    ensure_timezone_naive(value, arg_name)?;
    construct_naive_time(&components, value, arg_name).map(Some)
}

/// Wall-clock fields read from a Python `datetime.time`.
struct TimeComponents {
    /// Hour in the range 0-23.
    hour: u32,
    /// Minute in the range 0-59.
    minute: u32,
    /// Second in the range 0-59.
    second: u32,
    /// Microsecond in the range 0-999_999.
    microsecond: u32,
}

/// Report whether `None` is accepted, rejecting it when the caller disallows it.
fn is_allowed_none(value: &Bound<'_, PyAny>, arg_name: &str, allow_none: bool) -> PyResult<bool> {
    let is_none = value.is_none();
    if is_none && !allow_none {
        return Err(PyTypeError::new_err(format!(
            "{arg_name} must be datetime.time or None"
        )));
    }
    Ok(is_none)
}

/// Import the `datetime.time` class used to validate the supplied value.
fn python_time_type<'py>(value: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    value.py().import("datetime")?.getattr("time")
}

/// Reject values that are not instances of `datetime.time`.
fn ensure_is_time(
    value: &Bound<'_, PyAny>,
    time_type: &Bound<'_, PyAny>,
    arg_name: &str,
) -> PyResult<()> {
    if value.is_instance(time_type)? {
        return Ok(());
    }
    Err(PyTypeError::new_err(format!(
        "{arg_name} must be datetime.time, got {}",
        fq_py_type(value),
    )))
}

/// Read the wall-clock fields before any timezone check, preserving error order.
fn read_time_components(value: &Bound<'_, PyAny>) -> PyResult<TimeComponents> {
    Ok(TimeComponents {
        hour: value.getattr("hour")?.extract()?,
        minute: value.getattr("minute")?.extract()?,
        second: value.getattr("second")?.extract()?,
        microsecond: value.getattr("microsecond")?.extract()?,
    })
}

/// Reject timezone-aware values, matching the stdlib handler contract.
fn ensure_timezone_naive(value: &Bound<'_, PyAny>, arg_name: &str) -> PyResult<()> {
    if value.getattr("tzinfo")?.is_none() {
        return Ok(());
    }
    Err(PyValueError::new_err(format!(
        "{arg_name} must be timezone-naive"
    )))
}

/// Build the `NaiveTime`, reporting the original type error for invalid fields.
fn construct_naive_time(
    components: &TimeComponents,
    value: &Bound<'_, PyAny>,
    arg_name: &str,
) -> PyResult<NaiveTime> {
    NaiveTime::from_hms_micro_opt(
        components.hour,
        components.minute,
        components.second,
        components.microsecond,
    )
    .ok_or_else(|| {
        PyTypeError::new_err(format!(
            "invalid {arg_name} value of type {}",
            fq_py_type(value)
        ))
    })
}

/// Converts a Python datetime.time value using the common validation helper.
pub(super) fn extract_naive_time(value: &Bound<'_, PyAny>) -> PyResult<NaiveTime> {
    // Local convenience wrapper for the common helper; this retains the
    // existing `at_time`-specific error messages.
    extract_naive_time_from_py_time(value, "at_time", false)?.map_or_else(
        || {
            // `allow_none` is false, so the helper rejects None before
            // reaching this branch. A defensive error keeps the contract
            // visible.
            Err(PyTypeError::new_err(
                "at_time must be datetime.time or None",
            ))
        },
        Ok,
    )
}

#[cfg(test)]
mod tests {
    //! Tests for Python `datetime.time` extraction and error reporting.

    use chrono::NaiveTime;
    use pyo3::types::{PyAnyMethods, PyModule};
    use pyo3::{Bound, Python};

    use super::extract_naive_time_from_py_time;

    /// Attach to the GIL and run `case` with the `datetime` module loaded.
    fn with_datetime(case: impl FnOnce(&Bound<'_, PyModule>)) {
        Python::attach(|py| {
            let datetime = py.import("datetime").expect("import datetime");
            case(&datetime);
        });
    }

    #[test]
    fn accepts_none_when_allowed() {
        Python::attach(|py| {
            let none = py.None();
            let parsed = extract_naive_time_from_py_time(none.bind(py), "at_time", true)
                .expect("None must be accepted when allowed");
            assert!(parsed.is_none());
        });
    }

    #[test]
    fn rejects_none_when_disallowed() {
        Python::attach(|py| {
            let none = py.None();
            let err = extract_naive_time_from_py_time(none.bind(py), "at_time", false)
                .expect_err("None must be rejected when disallowed");
            assert_eq!(
                err.to_string(),
                "TypeError: at_time must be datetime.time or None"
            );
        });
    }

    #[test]
    fn extracts_a_timezone_naive_time() {
        with_datetime(|datetime| {
            let value = datetime
                .getattr("time")
                .expect("datetime.time")
                .call1((1, 2, 3, 4))
                .expect("construct time");
            let parsed = extract_naive_time_from_py_time(&value, "at_time", false)
                .expect("naive time must parse")
                .expect("value present");
            assert_eq!(
                parsed,
                NaiveTime::from_hms_micro_opt(1, 2, 3, 4).expect("valid time")
            );
        });
    }

    #[test]
    fn rejects_timezone_aware_time() {
        with_datetime(|datetime| {
            let utc = datetime
                .getattr("timezone")
                .expect("datetime.timezone")
                .getattr("utc")
                .expect("timezone.utc");
            let value = datetime
                .getattr("time")
                .expect("datetime.time")
                .call1((1, 2, 3, 4, utc))
                .expect("construct aware time");
            let err = extract_naive_time_from_py_time(&value, "at_time", false)
                .expect_err("aware time must be rejected");
            assert_eq!(
                err.to_string(),
                "ValueError: at_time must be timezone-naive"
            );
        });
    }
}
