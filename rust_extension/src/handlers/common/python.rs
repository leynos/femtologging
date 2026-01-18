//! Python bindings for common handler builder types.
//!
//! This module exposes Python APIs for the [`PyOverflowPolicy`] wrapper and
//! helper methods used by file-based handler builders.

use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use pyo3::{
    Bound, IntoPyObjectExt,
    class::basic::CompareOp,
    exceptions::{PyTypeError, PyValueError},
    prelude::*,
    types::{PyDict, PyString},
};

use super::{CommonBuilder, FileLikeBuilderState, FormatterConfig};
use crate::handlers::file::OverflowPolicy;

/// Python wrapper for [`OverflowPolicy`] providing factory methods and Python
/// protocol implementations.
#[pyclass(name = "OverflowPolicy")]
#[derive(Clone)]
pub struct PyOverflowPolicy {
    pub(crate) inner: OverflowPolicy,
}

#[pymethods]
impl PyOverflowPolicy {
    #[staticmethod]
    fn drop() -> Self {
        Self {
            inner: OverflowPolicy::Drop,
        }
    }

    #[staticmethod]
    fn block() -> Self {
        Self {
            inner: OverflowPolicy::Block,
        }
    }

    #[staticmethod]
    fn timeout(timeout_ms: u64) -> PyResult<Self> {
        if timeout_ms == 0 {
            return Err(PyValueError::new_err("timeout must be greater than zero"));
        }
        Ok(Self {
            inner: OverflowPolicy::Timeout(std::time::Duration::from_millis(timeout_ms)),
        })
    }

    fn __str__(&self) -> String {
        self.__repr__()
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            OverflowPolicy::Drop => "OverflowPolicy.drop()".to_string(),
            OverflowPolicy::Block => "OverflowPolicy.block()".to_string(),
            OverflowPolicy::Timeout(duration) => {
                format!("OverflowPolicy.timeout({})", duration.as_millis())
            }
        }
    }

    fn __richcmp__<'py>(&'py self, other: &Bound<'py, PyAny>, op: CompareOp) -> PyResult<bool> {
        let other_policy = other.extract::<PyRef<'py, PyOverflowPolicy>>().ok();

        match op {
            CompareOp::Eq => Ok(other_policy
                .map(|policy| self.inner == policy.inner)
                .unwrap_or(false)),
            CompareOp::Ne => Ok(other_policy
                .map(|policy| self.inner != policy.inner)
                .unwrap_or(true)),
            _ => Err(PyTypeError::new_err("ordering not supported")),
        }
    }

    fn __hash__(&self) -> PyResult<isize> {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);

        Ok(hasher.finish() as isize)
    }
}

impl CommonBuilder {
    /// Set the formatter from a Python object (string identifier or callable).
    pub fn set_formatter_from_py(&mut self, formatter: &Bound<'_, PyAny>) -> PyResult<()> {
        match formatter.downcast::<PyString>() {
            Ok(py_str) => {
                self.set_formatter(py_str.to_str()?.to_owned());
                Ok(())
            }
            Err(downcast_err) => match crate::formatter::python::formatter_from_py(formatter) {
                Ok(instance) => {
                    // The extracted formatter is already wrapped in a shared trait
                    // object; storing it directly avoids double `Arc` wrapping via the
                    // blanket `IntoFormatterConfig` implementation.
                    self.formatter = Some(FormatterConfig::Instance(instance));
                    Ok(())
                }
                Err(instance_err) => {
                    let py = formatter.py();

                    let string_err: PyErr = downcast_err.into();
                    let string_context =
                        PyTypeError::new_err("formatter string identifier extraction failed");
                    string_context.set_cause(py, Some(string_err));

                    if let Some(existing_cause) = instance_err.cause(py) {
                        let bound_cause = existing_cause.clone_ref(py).into_bound_py_any(py)?;
                        let callable_err = PyErr::from_value(bound_cause);
                        callable_err.set_cause(py, Some(string_context));
                        instance_err.set_cause(py, Some(callable_err));
                    } else {
                        instance_err.set_cause(py, Some(string_context));
                    }

                    Err(instance_err)
                }
            },
        }
    }

    /// Extend a Python dictionary with common builder fields.
    pub fn extend_py_dict(&self, d: &Bound<'_, PyDict>) -> PyResult<()> {
        if let Some(cap) = self.capacity {
            d.set_item("capacity", cap.get())?;
        }
        if let Some(ms) = self.flush_timeout_ms {
            d.set_item("flush_timeout_ms", ms.get())?;
        }
        if let Some(fmt) = &self.formatter {
            match fmt {
                FormatterConfig::Id(fid) => {
                    d.set_item("formatter_kind", "id")?;
                    d.set_item("formatter_id", fid.as_str())?;
                }
                FormatterConfig::Instance(_) => {
                    d.set_item("formatter_kind", "instance")?;
                    d.set_item("formatter", "instance")?;
                }
            }
        }
        Ok(())
    }
}

impl FileLikeBuilderState {
    /// Set the formatter from a Python object.
    pub fn set_formatter_from_py(&mut self, formatter: &Bound<'_, PyAny>) -> PyResult<()> {
        self.common.set_formatter_from_py(formatter)
    }

    /// Extend a Python dictionary with shared file builder fields.
    pub fn extend_py_dict(&self, d: &Bound<'_, PyDict>) -> PyResult<()> {
        self.common.extend_py_dict(d)?;
        if let Some(flush) = self.flush_record_interval {
            d.set_item("flush_record_interval", flush)?;
        }
        match self.overflow_policy {
            OverflowPolicy::Drop => d.set_item("overflow_policy", "drop")?,
            OverflowPolicy::Block => d.set_item("overflow_policy", "block")?,
            OverflowPolicy::Timeout(duration) => {
                d.set_item("timeout_ms", duration.as_millis() as u64)?;
                d.set_item("overflow_policy", "timeout")?;
            }
        }
        Ok(())
    }
}
