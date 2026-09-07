//! Python bindings for timed rotating handler APIs.
//!
//! This keeps `PyO3` details separate from the Rust core so configuration and
//! rotation logic stay testable without Python.

use chrono::NaiveTime;
use pyo3::{
    Bound,
    exceptions::{PyTypeError, PyValueError},
    prelude::*,
};

use super::{
    FemtoTimedRotatingFileHandler as CoreTimedRotatingFileHandler, TimedRotationConfig,
    schedule::{TimedRotationSchedule, TimedRotationWhen},
};

#[cfg(feature = "test-util")]
use super::clock::{clear_injected_times_for_test, set_injected_times_for_test};
use crate::{
    handlers::file::{self, DEFAULT_CHANNEL_CAPACITY, HandlerConfig},
    python::fq_py_type,
};

#[path = "python_constructor.rs"]
mod constructor;
#[cfg(test)]
#[path = "python_constructor_tests.rs"]
mod constructor_tests;

/// Python wrapper for the timed rotating file handler core type.
#[pyclass(name = "FemtoTimedRotatingFileHandler")]
pub struct PyTimedRotatingFileHandler {
    inner: CoreTimedRotatingFileHandler,
}

impl PyTimedRotatingFileHandler {
    pub(crate) const fn from_core(inner: CoreTimedRotatingFileHandler) -> Self {
        Self { inner }
    }
}

/// Error message describing supported timed rotation values.
pub const TIMED_ROTATION_VALIDATION_MSG: &str =
    "when must be one of: S, M, H, D, MIDNIGHT, or W0-W6";

/// Python options bundling queue and timed-rotation configuration.
#[pyclass(from_py_object, name = "TimedHandlerOptions")]
#[derive(Clone)]
pub struct TimedHandlerOptions {
    /// Queue capacity for the file handler.
    #[pyo3(get, set)]
    pub capacity: usize,
    /// Number of records written before a flush.
    #[pyo3(get, set)]
    pub flush_interval: isize,
    /// Overflow policy applied when the queue is full.
    #[pyo3(get, set)]
    pub policy: String,
    /// Rotation schedule selector.
    #[pyo3(get, set)]
    pub when: String,
    /// Number of schedule units between rotations.
    #[pyo3(get, set)]
    pub interval: u32,
    /// Number of rotated files retained after a rotation.
    #[pyo3(get, set)]
    pub backup_count: usize,
    /// Whether schedule calculations use UTC.
    #[pyo3(get, set)]
    pub utc: bool,
    at_time: Option<NaiveTime>,
}

impl TimedHandlerOptions {
    pub(crate) const fn at_time_naive(&self) -> Option<NaiveTime> {
        self.at_time
    }

    fn to_configs(&self) -> PyResult<(HandlerConfig, TimedRotationSchedule, usize)> {
        let capacity_input = isize::try_from(self.capacity)
            .map_err(|_| PyValueError::new_err("capacity must fit within isize"))?;
        let (validated_capacity, validated_flush_interval) = match self.flush_interval {
            -1 => file::validate_params(capacity_input, 1)?,
            value => file::validate_params(capacity_input, value)?,
        };
        let overflow_policy = file::policy::parse_policy_string(&self.policy)
            .map_err(|err| PyValueError::new_err(err.to_string()))?;
        let when = TimedRotationWhen::parse(&self.when).map_err(|err| {
            if err.starts_with("unsupported timed rotation value") {
                PyValueError::new_err(TIMED_ROTATION_VALIDATION_MSG)
            } else {
                PyValueError::new_err(err)
            }
        })?;
        let schedule = TimedRotationSchedule::new(when, self.interval, self.utc, self.at_time)
            .map_err(PyValueError::new_err)?;
        let config = HandlerConfig {
            capacity: validated_capacity,
            flush_interval: validated_flush_interval,
            overflow_policy,
        };
        Ok((config, schedule, self.backup_count))
    }
}

impl Default for TimedHandlerOptions {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_CHANNEL_CAPACITY,
            flush_interval: 1,
            policy: "drop".to_owned(),
            when: "H".to_owned(),
            interval: 1,
            backup_count: 0,
            utc: false,
            at_time: None,
        }
    }
}

#[path = "python_bindings.rs"]
mod python_bindings;

#[cfg(feature = "test-util")]
pub use python_bindings::{
    clear_timed_rotation_test_times_for_test, set_timed_rotation_test_times_for_test,
};

/// Extract a `NaiveTime` from a Python `datetime.time` object.
///
/// # Parameters
///
/// - `value`: The Python object to extract from
/// - `arg_name`: The name of the argument (for error messages)
/// - `allow_none`: If `true`, `None` values return `Ok(None)`; if `false`, they raise an error
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

fn extract_naive_time(value: &Bound<'_, PyAny>) -> PyResult<NaiveTime> {
    // Local convenience wrapper for the common helper; this retains the
    // existing `at_time`-specific error messages.
    extract_naive_time_from_py_time(value, "at_time", false)?.map_or_else(
        || {
            // `allow_none` is false, so the helper rejects None before reaching
            // this closure. A defensive error keeps the contract visible.
            Err(PyTypeError::new_err(
                "at_time must be datetime.time or None",
            ))
        },
        Ok,
    )
}
