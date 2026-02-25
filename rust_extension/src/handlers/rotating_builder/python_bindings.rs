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
        rotating::FemtoRotatingFileHandler,
    },
    macros::{AsPyDict, dict_into_py},
};

/// Extract a positive `i128` from a Python object.
fn extract_positive_i128(value: Bound<'_, PyAny>, field: &str) -> PyResult<i128> {
    let value = value.extract::<i128>()?;
    if value <= 0 {
        return Err(PyValueError::new_err(format!(
            "{field} must be greater than zero",
        )));
    }
    Ok(value)
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

#[pymethods]
impl RotatingFileHandlerBuilder {
    /// Create a new `RotatingFileHandlerBuilder`.
    #[new]
    fn py_new(path: String) -> Self {
        Self::new(path)
    }

    #[pyo3(name = "with_capacity")]
    #[pyo3(signature = (capacity))]
    #[pyo3(text_signature = "(self, capacity)")]
    fn py_with_capacity<'py>(
        mut slf: PyRefMut<'py, Self>,
        capacity: usize,
    ) -> PyResult<PyRefMut<'py, Self>> {
        if capacity == 0 {
            return Err(PyValueError::new_err("capacity must be greater than zero"));
        }
        slf.common.set_capacity(capacity);
        Ok(slf)
    }

    #[pyo3(name = "with_flush_after_records")]
    #[pyo3(signature = (interval))]
    #[pyo3(text_signature = "(self, interval)")]
    fn py_with_flush_after_records<'py>(
        mut slf: PyRefMut<'py, Self>,
        interval: u64,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let interval = py_flush_after_records_to_nonzero(interval)?;
        slf.common.set_flush_after_records(interval);
        Ok(slf)
    }

    #[pyo3(name = "with_max_bytes")]
    #[pyo3(signature = (max_bytes))]
    #[pyo3(text_signature = "(self, max_bytes)")]
    fn py_with_max_bytes<'py>(
        mut slf: PyRefMut<'py, Self>,
        max_bytes: Bound<'py, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let max_bytes = extract_positive_i128(max_bytes, "max_bytes")?;
        let max_bytes = u64::try_from(max_bytes)
            .map_err(|_| PyOverflowError::new_err("max_bytes exceeds the allowable range"))?;
        slf.max_bytes = NonZeroU64::new(max_bytes);
        slf.max_bytes_set = true;
        Ok(slf)
    }

    #[pyo3(name = "with_backup_count")]
    #[pyo3(signature = (backup_count))]
    #[pyo3(text_signature = "(self, backup_count)")]
    fn py_with_backup_count<'py>(
        mut slf: PyRefMut<'py, Self>,
        backup_count: Bound<'py, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let backup_count = extract_positive_i128(backup_count, "backup_count")?;
        let backup_count = usize::try_from(backup_count)
            .map_err(|_| PyOverflowError::new_err("backup_count exceeds the allowable range"))?;
        slf.backup_count = NonZeroUsize::new(backup_count);
        slf.backup_count_set = true;
        Ok(slf)
    }

    #[pyo3(name = "with_overflow_policy")]
    fn py_with_overflow_policy<'py>(
        mut slf: PyRefMut<'py, Self>,
        policy: PyOverflowPolicy,
    ) -> PyResult<PyRefMut<'py, Self>> {
        slf.common.set_overflow_policy(policy.inner);
        Ok(slf)
    }

    #[pyo3(name = "with_formatter")]
    #[pyo3(signature = (formatter))]
    #[pyo3(text_signature = "(self, formatter)")]
    fn py_with_formatter<'py>(
        mut slf: PyRefMut<'py, Self>,
        formatter: Bound<'py, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        slf.common.set_formatter_from_py(&formatter)?;
        Ok(slf)
    }

    /// Return a dictionary describing the builder configuration.
    fn as_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.as_pydict(py)
    }

    /// Build the handler, raising ``HandlerConfigError`` or ``HandlerIOError`` on
    /// failure.
    fn build(&self) -> PyResult<FemtoRotatingFileHandler> {
        <Self as HandlerBuilderTrait>::build_inner(self).map_err(PyErr::from)
    }
}

impl AsPyDict for RotatingFileHandlerBuilder {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let d = PyDict::new(py);
        fill_pydict(self, &d)?;
        dict_into_py(d, py)
    }
}
