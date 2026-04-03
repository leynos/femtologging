//! Python bindings for timed rotating handler APIs.
//!
//! This keeps PyO3 details separate from the Rust core so configuration and
//! rotation logic stay testable without Python.

use chrono::NaiveTime;
use pyo3::{
    Bound,
    exceptions::{PyIOError, PyTypeError, PyValueError},
    prelude::*,
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

/// Python wrapper for the timed rotating file handler core type.
#[pyclass(name = "FemtoTimedRotatingFileHandler")]
pub struct PyTimedRotatingFileHandler {
    inner: CoreTimedRotatingFileHandler,
}

impl PyTimedRotatingFileHandler {
    pub(crate) fn from_core(inner: CoreTimedRotatingFileHandler) -> Self {
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
    #[pyo3(get, set)]
    pub capacity: usize,
    #[pyo3(get, set)]
    pub flush_interval: isize,
    #[pyo3(get, set)]
    pub policy: String,
    #[pyo3(get, set)]
    pub when: String,
    #[pyo3(get, set)]
    pub interval: u32,
    #[pyo3(get, set)]
    pub backup_count: usize,
    #[pyo3(get, set)]
    pub utc: bool,
    at_time: Option<NaiveTime>,
}

impl TimedHandlerOptions {
    pub(crate) fn at_time_naive(&self) -> Option<NaiveTime> {
        self.at_time
    }

    fn to_configs(&self) -> PyResult<(HandlerConfig, TimedRotationSchedule, usize)> {
        let capacity = isize::try_from(self.capacity)
            .map_err(|_| PyValueError::new_err("capacity must be greater than zero"))?;
        let (capacity, flush_interval) = match self.flush_interval {
            -1 => file::validate_params(capacity, 1)?,
            value => file::validate_params(capacity, value)?,
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
            capacity,
            flush_interval,
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
            policy: "drop".to_string(),
            when: "H".to_string(),
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
    #[pyo3(
        text_signature = "(capacity=DEFAULT_CHANNEL_CAPACITY, flush_interval=1, policy='drop', when='H', interval=1, backup_count=0, utc=False, at_time=None)"
    )]
    #[pyo3(signature = (
        capacity = DEFAULT_CHANNEL_CAPACITY,
        flush_interval = 1,
        policy = "drop".to_string(),
        when = "H".to_string(),
        interval = 1,
        backup_count = 0,
        utc = false,
        at_time = None,
    ))]
    fn new(
        capacity: usize,
        flush_interval: isize,
        policy: String,
        when: String,
        interval: u32,
        backup_count: usize,
        utc: bool,
        at_time: Option<Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let at_time = match at_time {
            Some(value) => Some(extract_naive_time(value)?),
            None => None,
        };
        let options = Self {
            capacity,
            flush_interval,
            policy,
            when,
            interval,
            backup_count,
            utc,
            at_time,
        };
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
    fn py_new(path: String, options: Option<TimedHandlerOptions>) -> PyResult<Self> {
        let options = options.unwrap_or_default();
        let (config, schedule, backup_count) = options.to_configs()?;
        CoreTimedRotatingFileHandler::with_capacity_flush_policy(
            &path,
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
    fn when(&self) -> &str {
        self.inner.schedule().when().as_str()
    }

    #[getter]
    fn interval(&self) -> u32 {
        self.inner.schedule().interval()
    }

    #[getter]
    fn backup_count(&self) -> usize {
        self.inner.backup_count()
    }

    #[getter]
    fn utc(&self) -> bool {
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

fn extract_naive_time(value: Bound<'_, PyAny>) -> PyResult<NaiveTime> {
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
