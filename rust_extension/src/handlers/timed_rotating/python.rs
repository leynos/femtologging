//! Python bindings for timed rotating handler APIs.
//!
//! This keeps `PyO3` details separate from the Rust core so configuration and
//! rotation logic stay testable without Python.

use chrono::NaiveTime;
use pyo3::{
    Bound,
    exceptions::{PyIOError, PyTypeError, PyValueError},
    prelude::*,
    types::{PyDict, PyTuple},
};

use super::{
    FemtoTimedRotatingFileHandler as CoreTimedRotatingFileHandler, TimedRotationConfig,
    schedule::{TimedRotationSchedule, TimedRotationWhen},
};

#[cfg(feature = "test-util")]
use super::clock::{clear_injected_times_for_test, set_injected_times_for_test};
use crate::{
    formatter::DefaultFormatter,
    handler::FemtoHandlerTrait,
    handlers::file::{self, DEFAULT_CHANNEL_CAPACITY, HandlerConfig},
    level::FemtoLevel,
    log_record::FemtoLogRecord,
    python::fq_py_type,
};

#[path = "python_constructor.rs"]
mod constructor;
#[cfg(test)]
#[path = "python_constructor_tests.rs"]
mod constructor_tests;

use constructor::TimedHandlerOptionsRequest;

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

#[pymethods]
impl TimedHandlerOptions {
    #[new]
    #[pyo3(signature = (*args, **kwargs))]
    #[pyo3(
        text_signature = "(capacity=DEFAULT_CHANNEL_CAPACITY, flush_interval=1, policy='drop', when='H', interval=1, backup_count=0, utc=False, at_time=None)"
    )]
    fn new(args: &Bound<'_, PyTuple>, kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
        let request = TimedHandlerOptionsRequest::from_python(args, kwargs)?;
        let options = Self::from(request);
        let _ = options.to_configs()?;
        Ok(options)
    }

    #[getter]
    fn at_time(&self) -> Option<String> {
        self.at_time.map(|value| value.to_string())
    }
}

#[pymethods]
impl PyTimedRotatingFileHandler {
    #[new]
    #[pyo3(text_signature = "(path, options=None)")]
    #[pyo3(signature = (path, options = None))]
    fn py_new(path: &str, options: Option<TimedHandlerOptions>) -> PyResult<Self> {
        let handler_options = options.unwrap_or_default();
        let (config, schedule, backup_count) = handler_options.to_configs()?;
        CoreTimedRotatingFileHandler::with_capacity_flush_policy(
            path,
            DefaultFormatter,
            config,
            TimedRotationConfig {
                schedule,
                backup_count,
            },
        )
        .map(Self::from_core)
        .map_err(|err| PyIOError::new_err(format!("{path}: {err}")))
    }

    #[getter]
    const fn when(&self) -> &str {
        self.inner.schedule().when().as_str()
    }

    #[getter]
    const fn interval(&self) -> u32 {
        self.inner.schedule().interval()
    }

    #[getter]
    const fn backup_count(&self) -> usize {
        self.inner.backup_count()
    }

    #[getter]
    const fn utc(&self) -> bool {
        self.inner.schedule().use_utc()
    }

    #[getter]
    fn at_time(&self) -> Option<String> {
        self.inner
            .schedule()
            .at_time()
            .map(|value| value.to_string())
    }

    #[pyo3(name = "handle")]
    fn py_handle(&self, logger: &str, level: &str, message: &str) -> PyResult<()> {
        let parsed_level = FemtoLevel::parse_py(level)?;
        self.inner
            .handle(FemtoLogRecord::new(logger, parsed_level, message))
            .map_err(|err| PyValueError::new_err(format!("Handler error: {err}")))
    }

    #[pyo3(name = "flush")]
    fn py_flush(&self) -> bool {
        self.inner.flush()
    }

    #[pyo3(name = "close")]
    fn py_close(&mut self) {
        self.inner.close();
    }
}

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

#[cfg(feature = "test-util")]
#[pyfunction]
pub fn set_timed_rotation_test_times_for_test(epoch_millis: Vec<i64>) {
    set_injected_times_for_test(epoch_millis);
}

#[cfg(feature = "test-util")]
#[pyfunction]
pub fn clear_timed_rotation_test_times_for_test() {
    clear_injected_times_for_test();
}
