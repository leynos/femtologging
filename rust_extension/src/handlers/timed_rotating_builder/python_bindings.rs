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
        file::policy::parse_policy_string,
        timed_rotating::{
            PyTimedRotatingFileHandler, TimedRotationWhen,
            python::{TimedHandlerOptions, extract_naive_time_from_py_time},
        },
    },
    macros::{AsPyDict, dict_into_py},
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
    extract_naive_time_from_py_time(&value, "at_time", true)
}

fn fill_pydict(builder: &TimedRotatingFileHandlerBuilder, d: &Bound<'_, PyDict>) -> PyResult<()> {
    d.set_item("path", builder.path.to_string_lossy().as_ref())?;
    builder.common.extend_py_dict(d)?;
    d.set_item("when", builder.when.as_str())?;
    d.set_item("interval", builder.interval.get())?;
    d.set_item("backup_count", builder.backup_count)?;
    d.set_item("utc", builder.use_utc)?;
    if let Some(at_time) = builder.at_time {
        d.set_item("at_time", at_time.to_string())?;
    }
    Ok(())
}

fn apply_builder_update<'py, F>(
    mut slf: PyRefMut<'py, TimedRotatingFileHandlerBuilder>,
    update: F,
) -> PyResult<PyRefMut<'py, TimedRotatingFileHandlerBuilder>>
where
    F: FnOnce(&mut TimedRotatingFileHandlerBuilder) -> PyResult<()>,
{
    update(&mut slf)?;
    Ok(slf)
}

#[pymethods]
impl TimedRotatingFileHandlerBuilder {
    #[new]
    #[pyo3(signature = (path, options = None))]
    fn py_new(path: String, options: Option<Bound<'_, TimedHandlerOptions>>) -> PyResult<Self> {
        let mut builder = Self::new(path);
        let Some(opts) = options else {
            return Ok(builder);
        };
        let opts_ref = opts.borrow();

        // Apply queue configuration
        builder.common.set_capacity(opts_ref.capacity);
        let flush_interval_u64 = u64::try_from(opts_ref.flush_interval).map_err(|_| {
            PyOverflowError::new_err(format!(
                "flush_interval out of range for u64: {}",
                opts_ref.flush_interval
            ))
        })?;
        let flush_interval = py_flush_after_records_to_nonzero(flush_interval_u64)?;
        builder.common.set_flush_after_records(flush_interval);
        let policy = parse_policy_string(&opts_ref.policy)
            .map_err(|err| PyValueError::new_err(err.to_string()))?;
        builder.common.set_overflow_policy(policy);

        // Apply rotation configuration
        let when = opts_ref.when.clone();
        let interval = nonzero_interval(u64::from(opts_ref.interval))?;
        let at_time_naive = opts_ref.at_time_naive();
        let backup_count = opts_ref.backup_count;
        let utc = opts_ref.utc;

        builder
            .with_when(when)
            .map_err(map_config_error)?
            .with_at_time(at_time_naive)
            .map_err(map_config_error)
            .map(|b| {
                b.with_interval(interval)
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
        apply_builder_update(slf, |builder| {
            builder.common.set_capacity(capacity);
            Ok(())
        })
    }

    #[pyo3(name = "with_flush_after_records")]
    fn py_with_flush_after_records<'py>(
        slf: PyRefMut<'py, Self>,
        interval: u64,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let interval = py_flush_after_records_to_nonzero(interval)?;
        apply_builder_update(slf, |builder| {
            builder.common.set_flush_after_records(interval);
            Ok(())
        })
    }

    #[pyo3(name = "with_when")]
    fn py_with_when<'py>(slf: PyRefMut<'py, Self>, when: String) -> PyResult<PyRefMut<'py, Self>> {
        apply_builder_update(slf, |builder| {
            let when = TimedRotationWhen::parse(&when)
                .map_err(HandlerBuildError::InvalidConfig)
                .map_err(map_config_error)?;
            if builder.at_time.is_some() && !when.supports_at_time() {
                return Err(map_config_error(HandlerBuildError::InvalidConfig(format!(
                    "at_time is only supported for daily, midnight, and weekday rotation (got {})",
                    when.as_str(),
                ))));
            }
            // Validate weekday/interval invariant
            if matches!(when, TimedRotationWhen::Weekday(_)) && builder.interval.get() != 1 {
                return Err(map_config_error(HandlerBuildError::InvalidConfig(
                    "weekday rotation only supports interval = 1".to_string(),
                )));
            }
            builder.when = when;
            Ok(())
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
        let interval = nonzero_interval(interval)?;
        apply_builder_update(slf, |builder| {
            // Validate weekday/interval invariant
            if matches!(builder.when, TimedRotationWhen::Weekday(_)) && interval.get() != 1 {
                return Err(map_config_error(HandlerBuildError::InvalidConfig(
                    "weekday rotation only supports interval = 1".to_string(),
                )));
            }
            builder.interval = interval;
            Ok(())
        })
    }

    #[pyo3(name = "with_backup_count")]
    fn py_with_backup_count<'py>(
        slf: PyRefMut<'py, Self>,
        backup_count: Bound<'py, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let backup_count = backup_count.extract::<usize>()?;
        apply_builder_update(slf, |builder| {
            builder.backup_count = backup_count;
            Ok(())
        })
    }

    #[pyo3(name = "with_utc")]
    fn py_with_utc<'py>(slf: PyRefMut<'py, Self>, use_utc: bool) -> PyResult<PyRefMut<'py, Self>> {
        apply_builder_update(slf, |builder| {
            builder.use_utc = use_utc;
            Ok(())
        })
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
            if at_time.is_some() && !builder.when.supports_at_time() {
                return Err(map_config_error(HandlerBuildError::InvalidConfig(format!(
                    "at_time is only supported for daily, midnight, and weekday rotation (got {})",
                    builder.when.as_str(),
                ))));
            }
            builder.at_time = at_time;
            Ok(())
        })
    }

    #[pyo3(name = "with_overflow_policy")]
    fn py_with_overflow_policy<'py>(
        slf: PyRefMut<'py, Self>,
        policy: PyOverflowPolicy,
    ) -> PyResult<PyRefMut<'py, Self>> {
        apply_builder_update(slf, |builder| {
            builder.common.set_overflow_policy(policy.inner);
            Ok(())
        })
    }

    #[pyo3(name = "with_formatter")]
    fn py_with_formatter<'py>(
        slf: PyRefMut<'py, Self>,
        formatter: Bound<'py, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        apply_builder_update(slf, |builder| {
            builder.common.set_formatter_from_py(&formatter)?;
            Ok(())
        })
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
