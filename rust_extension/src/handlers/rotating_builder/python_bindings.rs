//! Python bindings for [`RotatingFileHandlerBuilder`].
//!
//! This module keeps PyO3-specific wrappers separate from the core builder
//! logic so the Rust implementation remains focused on validation and handler
//! construction.

use std::num::{NonZeroU64, NonZeroUsize};

use pyo3::{
    exceptions::{PyOverflowError, PyValueError},
    prelude::*,
    types::{PyAny, PyDict},
};

use super::RotatingFileHandlerBuilder;
use crate::{
    handlers::{
        HandlerBuilderTrait,
        common::{PyOverflowPolicy, py_flush_after_records_to_nonzero},
        file::OverflowPolicy,
        rotating::PyRotatingFileHandler,
    },
    macros::{AsPyDict, dict_into_py},
};

/// Extract a positive `i128` from a Python-supplied builder value.
fn extract_positive_i128(value: i128, field: &str) -> PyResult<i128> {
    if value <= 0 {
        return Err(PyValueError::new_err(format!(
            "{field} must be greater than zero",
        )));
    }
    Ok(value)
}

/// Extract the overflow policy from its Python wrapper object.
fn extract_overflow_policy(policy: &Bound<'_, PyAny>) -> PyResult<OverflowPolicy> {
    let wrapper = policy.extract::<PyRef<'_, PyOverflowPolicy>>()?;
    Ok(wrapper.inner)
}

/// Populate a Python dictionary with the builder's fields.
fn fill_pydict(builder: &RotatingFileHandlerBuilder, d: &Bound<'_, PyDict>) -> PyResult<()> {
    d.set_item("path", &builder.path)?;
    builder.common.extend_py_dict(d)?;
    d.set_item("max_bytes", builder.max_bytes.map_or(0, NonZeroU64::get))?;
    d.set_item(
        "backup_count",
        builder.backup_count.map_or(0, NonZeroUsize::get),
    )?;
    Ok(())
}

/// Apply an immutable builder update to a borrowed `PyO3` builder reference.
fn apply_builder_update<F>(
    mut slf: PyRefMut<'_, RotatingFileHandlerBuilder>,
    update: F,
) -> PyResult<PyRefMut<'_, RotatingFileHandlerBuilder>>
where
    F: FnOnce(RotatingFileHandlerBuilder) -> PyResult<RotatingFileHandlerBuilder>,
{
    *slf = update(slf.clone())?;
    Ok(slf)
}

#[pymethods]
impl RotatingFileHandlerBuilder {
    /// Create a new `RotatingFileHandlerBuilder`.
    #[new]
    fn py_new(path: String) -> Self {
        Self::new(path)
    }

    /// Set the builder capacity.
    #[pyo3(name = "with_capacity")]
    #[pyo3(signature = (capacity))]
    #[pyo3(text_signature = "(self, capacity)")]
    fn py_with_capacity(slf: PyRefMut<'_, Self>, capacity: i128) -> PyResult<PyRefMut<'_, Self>> {
        let positive_capacity = extract_positive_i128(capacity, "capacity")?;
        let validated_capacity = usize::try_from(positive_capacity)
            .map_err(|_| PyOverflowError::new_err("capacity exceeds the allowable range"))?;
        apply_builder_update(slf, |builder| Ok(builder.with_capacity(validated_capacity)))
    }

    /// Set the flush interval in records.
    #[pyo3(name = "with_flush_after_records")]
    #[pyo3(signature = (interval))]
    #[pyo3(text_signature = "(self, interval)")]
    fn py_with_flush_after_records(
        slf: PyRefMut<'_, Self>,
        interval: u64,
    ) -> PyResult<PyRefMut<'_, Self>> {
        let validated_interval = py_flush_after_records_to_nonzero(interval)?;
        apply_builder_update(slf, |builder| {
            Ok(builder.with_flush_after_records(validated_interval))
        })
    }

    /// Set the maximum bytes threshold for rotation.
    #[pyo3(name = "with_max_bytes")]
    #[pyo3(signature = (max_bytes))]
    #[pyo3(text_signature = "(self, max_bytes)")]
    fn py_with_max_bytes(slf: PyRefMut<'_, Self>, max_bytes: i128) -> PyResult<PyRefMut<'_, Self>> {
        let positive_max_bytes = extract_positive_i128(max_bytes, "max_bytes")?;
        let validated_max_bytes = u64::try_from(positive_max_bytes)
            .map_err(|_| PyOverflowError::new_err("max_bytes exceeds the allowable range"))?;
        apply_builder_update(slf, |builder| {
            Ok(builder.with_max_bytes(validated_max_bytes))
        })
    }

    /// Set the backup-file count for rotation.
    #[pyo3(name = "with_backup_count")]
    #[pyo3(signature = (backup_count))]
    #[pyo3(text_signature = "(self, backup_count)")]
    fn py_with_backup_count(
        slf: PyRefMut<'_, Self>,
        backup_count: i128,
    ) -> PyResult<PyRefMut<'_, Self>> {
        let positive_backup_count = extract_positive_i128(backup_count, "backup_count")?;
        let validated_backup_count = usize::try_from(positive_backup_count)
            .map_err(|_| PyOverflowError::new_err("backup_count exceeds the allowable range"))?;
        apply_builder_update(slf, |builder| {
            Ok(builder.with_backup_count(validated_backup_count))
        })
    }

    /// Set the overflow policy for queue saturation.
    #[pyo3(name = "with_overflow_policy")]
    #[pyo3(signature = (policy))]
    fn py_with_overflow_policy(
        slf: PyRefMut<'_, Self>,
        #[pyo3(from_py_with = extract_overflow_policy)] policy: OverflowPolicy,
    ) -> PyResult<PyRefMut<'_, Self>> {
        apply_builder_update(slf, |builder| Ok(builder.with_overflow_policy(policy)))
    }

    /// Set a formatter for the rotating file builder.
    #[pyo3(name = "with_formatter")]
    #[pyo3(signature = (formatter))]
    #[pyo3(text_signature = "(self, formatter)")]
    fn py_with_formatter<'py>(
        slf: PyRefMut<'py, Self>,
        formatter: Bound<'py, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let formatter_any = formatter.into_any();
        apply_builder_update(slf, |builder| {
            builder.with_formatter_from_py(&formatter_any)
        })
    }

    /// Store filter IDs for resolution by the configuration builder.
    #[pyo3(name = "with_filters")]
    fn py_with_filters(
        slf: PyRefMut<'_, Self>,
        filter_ids: Vec<String>,
    ) -> PyResult<PyRefMut<'_, Self>> {
        apply_builder_update(slf, |builder| Ok(builder.with_filters(filter_ids)))
    }

    /// Return a dictionary describing the builder configuration.
    fn as_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.as_pydict(py)
    }

    /// Build the handler, raising ``HandlerConfigError`` or ``HandlerIOError`` on
    /// failure.
    fn build(&self) -> PyResult<PyRotatingFileHandler> {
        <Self as HandlerBuilderTrait>::build_inner(self)
            .map(PyRotatingFileHandler::from_core)
            .map_err(PyErr::from)
    }
}

impl AsPyDict for RotatingFileHandlerBuilder {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let d = PyDict::new(py);
        fill_pydict(self, &d)?;
        dict_into_py(d, py)
    }
}
