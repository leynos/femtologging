//! Python bindings for [`TimedRotatingFileHandlerBuilder`].
//!
//! The Rust builder remains the source of truth; this module only adapts
//! Python inputs and serialisation helpers.

use std::num::NonZeroU64;

use chrono::NaiveTime;
use pyo3::{
    Bound,
    exceptions::{PyOverflowError, PyValueError},
    prelude::*,
    types::{PyAny, PyDict},
};

use super::TimedRotatingFileHandlerBuilder;
use crate::{
    handlers::{
        HandlerBuildError, HandlerBuilderTrait,
        common::{PyOverflowPolicy, py_flush_after_records_to_nonzero},
        timed_rotating::PyTimedRotatingFileHandler,
    },
    macros::{AsPyDict, dict_into_py},
    python::fq_py_type,
};

fn extract_positive_i128(value: Bound<'_, PyAny>, field: &str) -> PyResult<i128> {
    let value = value.extract::<i128>()?;
    if value <= 0 {
        return Err(PyValueError::new_err(format!(
            "{field} must be greater than zero",
        )));
    }
    Ok(value)
}

fn nonzero_interval(value: u64) -> PyResult<NonZeroU64> {
    NonZeroU64::new(value)
        .ok_or_else(|| PyValueError::new_err("interval must be greater than zero"))
}

fn map_config_error(err: HandlerBuildError) -> PyErr {
    match err {
        HandlerBuildError::InvalidConfig(message) => PyValueError::new_err(message),
        other => PyErr::from(other),
    }
}

fn extract_optional_time(value: Bound<'_, PyAny>) -> PyResult<Option<NaiveTime>> {
    if value.is_none() {
        return Ok(None);
    }
    let hour: u32 = value.getattr("hour")?.extract()?;
    let minute: u32 = value.getattr("minute")?.extract()?;
    let second: u32 = value.getattr("second")?.extract()?;
    let microsecond: u32 = value.getattr("microsecond")?.extract()?;
    let tzinfo = value.getattr("tzinfo")?;
    if !tzinfo.is_none() {
        return Err(PyValueError::new_err("at_time must be timezone-naive"));
    }
    NaiveTime::from_hms_micro_opt(hour, minute, second, microsecond)
        .map(Some)
        .ok_or_else(|| {
            PyValueError::new_err(format!(
                "invalid at_time value of type {}",
                fq_py_type(&value)
            ))
        })
}

fn fill_pydict(builder: &TimedRotatingFileHandlerBuilder, d: &Bound<'_, PyDict>) -> PyResult<()> {
    d.set_item("path", builder.path.to_string_lossy().as_ref())?;
    builder.common.extend_py_dict(d)?;
    d.set_item("when", builder.when.as_str())?;
    d.set_item("interval", builder.interval.get())?;
    d.set_item("backup_count", builder.backup_count)?;
    d.set_item("utc", builder.use_utc)?;
    if let Some(at_time) = builder.at_time {
        d.set_item("at_time", at_time.format("%H:%M:%S").to_string())?;
    }
    Ok(())
}

fn apply_builder_update<'py, F>(
    mut slf: PyRefMut<'py, TimedRotatingFileHandlerBuilder>,
    update: F,
) -> PyResult<PyRefMut<'py, TimedRotatingFileHandlerBuilder>>
where
    F: FnOnce(TimedRotatingFileHandlerBuilder) -> PyResult<TimedRotatingFileHandlerBuilder>,
{
    *slf = update(slf.clone())?;
    Ok(slf)
}

#[pymethods]
impl TimedRotatingFileHandlerBuilder {
    #[new]
    #[pyo3(signature = (
        path,
        when = "H".to_string(),
        interval = 1,
        backup_count = 0,
        utc = false,
        at_time = None,
    ))]
    fn py_new(
        path: String,
        when: String,
        interval: u64,
        backup_count: usize,
        utc: bool,
        at_time: Option<Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let interval = nonzero_interval(interval)?;
        let at_time = match at_time {
            Some(value) => extract_optional_time(value)?,
            None => None,
        };
        Self::new(path)
            .with_when(when)
            .map_err(map_config_error)?
            .with_at_time(at_time)
            .map_err(map_config_error)
            .map(|builder| {
                builder
                    .with_interval(interval)
                    .with_backup_count(backup_count)
                    .with_utc(utc)
            })
    }

    #[pyo3(name = "with_capacity")]
    fn py_with_capacity<'py>(
        slf: PyRefMut<'py, Self>,
        capacity: Bound<'py, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let capacity = extract_positive_i128(capacity, "capacity")?;
        let capacity = usize::try_from(capacity)
            .map_err(|_| PyOverflowError::new_err("capacity exceeds the allowable range"))?;
        apply_builder_update(slf, |builder| Ok(builder.with_capacity(capacity)))
    }

    #[pyo3(name = "with_flush_after_records")]
    fn py_with_flush_after_records<'py>(
        slf: PyRefMut<'py, Self>,
        interval: u64,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let interval = py_flush_after_records_to_nonzero(interval)?;
        apply_builder_update(
            slf,
            |builder| Ok(builder.with_flush_after_records(interval)),
        )
    }

    #[pyo3(name = "with_when")]
    fn py_with_when<'py>(slf: PyRefMut<'py, Self>, when: String) -> PyResult<PyRefMut<'py, Self>> {
        apply_builder_update(slf, |builder| {
            builder.with_when(when).map_err(map_config_error)
        })
    }

    #[pyo3(name = "with_interval")]
    fn py_with_interval<'py>(
        slf: PyRefMut<'py, Self>,
        interval: Bound<'py, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let interval = extract_positive_i128(interval, "interval")?;
        let interval = u64::try_from(interval)
            .map_err(|_| PyOverflowError::new_err("interval exceeds the allowable range"))?;
        let interval = NonZeroU64::new(interval)
            .ok_or_else(|| PyValueError::new_err("interval must be greater than zero"))?;
        apply_builder_update(slf, |builder| Ok(builder.with_interval(interval)))
    }

    #[pyo3(name = "with_backup_count")]
    fn py_with_backup_count<'py>(
        slf: PyRefMut<'py, Self>,
        backup_count: Bound<'py, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let backup_count = backup_count.extract::<usize>()?;
        apply_builder_update(slf, |builder| Ok(builder.with_backup_count(backup_count)))
    }

    #[pyo3(name = "with_utc")]
    fn py_with_utc<'py>(slf: PyRefMut<'py, Self>, use_utc: bool) -> PyResult<PyRefMut<'py, Self>> {
        apply_builder_update(slf, |builder| Ok(builder.with_utc(use_utc)))
    }

    #[pyo3(name = "with_at_time")]
    fn py_with_at_time<'py>(
        slf: PyRefMut<'py, Self>,
        at_time: Option<Bound<'py, PyAny>>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let at_time = match at_time {
            Some(value) => extract_optional_time(value)?,
            None => None,
        };
        apply_builder_update(slf, |builder| {
            builder.with_at_time(at_time).map_err(map_config_error)
        })
    }

    #[pyo3(name = "with_overflow_policy")]
    fn py_with_overflow_policy<'py>(
        slf: PyRefMut<'py, Self>,
        policy: PyOverflowPolicy,
    ) -> PyResult<PyRefMut<'py, Self>> {
        apply_builder_update(
            slf,
            |builder| Ok(builder.with_overflow_policy(policy.inner)),
        )
    }

    #[pyo3(name = "with_formatter")]
    fn py_with_formatter<'py>(
        slf: PyRefMut<'py, Self>,
        formatter: Bound<'py, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        apply_builder_update(slf, |builder| builder.with_formatter_from_py(&formatter))
    }

    fn as_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.as_pydict(py)
    }

    fn build(&self) -> PyResult<PyTimedRotatingFileHandler> {
        <Self as HandlerBuilderTrait>::build_inner(self)
            .map(PyTimedRotatingFileHandler::from_core)
            .map_err(PyErr::from)
    }
}

impl AsPyDict for TimedRotatingFileHandlerBuilder {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let d = PyDict::new(py);
        fill_pydict(self, &d)?;
        dict_into_py(d, py)
    }
}
