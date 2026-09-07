//! `PyO3` wrappers for the timed rotating handler types and test controls.
//!
//! The parent module owns schedule validation and rotation logic; this private
//! module contains only Python-facing macro expansions and their adapters.

#![expect(
    clippy::too_many_arguments,
    reason = "PyO3 generates five-argument Python call wrappers"
)]

use pyo3::{
    exceptions::{PyIOError, PyValueError},
    prelude::*,
    types::{PyDict, PyTuple},
};

use super::{
    CoreTimedRotatingFileHandler,
    PyTimedRotatingFileHandler,
    TimedHandlerOptions,
    TimedRotationConfig,
    constructor::TimedHandlerOptionsRequest,
};
use crate::{
    formatter::DefaultFormatter,
    handler::FemtoHandlerTrait,
    level::FemtoLevel,
    log_record::FemtoLogRecord,
};

#[pymethods]
impl TimedHandlerOptions {
    #[new]
    #[pyo3(signature = (*args, **kwargs))]
    #[pyo3(
        text_signature = "(capacity=DEFAULT_CHANNEL_CAPACITY, flush_interval=1, policy='drop', \
                          when='H', interval=1, backup_count=0, utc=False, at_time=None)"
    )]
    fn new(args: &Bound<'_, PyTuple>, kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
        let request = TimedHandlerOptionsRequest::from_python(args, kwargs)?;
        let options = Self::from(request);
        let _ = options.to_configs()?;
        Ok(options)
    }

    #[getter]
    fn at_time(&self) -> Option<String> { self.at_time.map(|value| value.to_string()) }
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
    const fn when(&self) -> &str { self.inner.schedule().when().as_str() }

    #[getter]
    const fn interval(&self) -> u32 { self.inner.schedule().interval() }

    #[getter]
    const fn backup_count(&self) -> usize { self.inner.backup_count() }

    #[getter]
    const fn utc(&self) -> bool { self.inner.schedule().use_utc() }

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
    fn py_flush(&self) -> bool { self.inner.flush() }

    #[pyo3(name = "close")]
    fn py_close(&mut self) { self.inner.close(); }
}

#[cfg(feature = "test-util")]
#[pyfunction]
pub fn set_timed_rotation_test_times_for_test(epoch_millis: Vec<i64>) {
    super::set_injected_times_for_test(epoch_millis);
}

#[cfg(feature = "test-util")]
#[pyfunction]
pub fn clear_timed_rotation_test_times_for_test() { super::clear_injected_times_for_test(); }
