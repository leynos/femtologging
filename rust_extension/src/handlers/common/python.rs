//! Python bindings for common handler builder types.
//!
//! This module exposes Python APIs for the [`PyOverflowPolicy`] wrapper and
//! helper methods used by file-based handler builders.

use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use std::num::NonZeroU64;

use pyo3::{
    Bound,
    class::basic::CompareOp,
    exceptions::{PyOverflowError, PyTypeError, PyValueError},
    prelude::*,
    types::{PyDict, PyString},
};

/// Convert a Python `u64` flush-after-records threshold to `NonZeroU64`.
///
/// Raises `ValueError` if the value is zero, or `OverflowError` if the
/// value exceeds `usize::MAX` on the current platform (since `HandlerConfig`
/// stores flush intervals as `usize` internally).
pub(crate) fn py_flush_after_records_to_nonzero(interval: u64) -> PyResult<NonZeroU64> {
    const FIELD: &str = "flush_after_records";
    if interval > usize::MAX as u64 {
        return Err(PyOverflowError::new_err(format!(
            "{FIELD} exceeds maximum value for this platform ({max})",
            max = usize::MAX,
        )));
    }
    NonZeroU64::new(interval)
        .ok_or_else(|| PyValueError::new_err(format!("{FIELD} must be greater than zero")))
}

use super::{CommonBuilder, FileLikeBuilderState, FormatterConfig};
use crate::handlers::file::OverflowPolicy;

/// Format an [`OverflowPolicy`] for Python `__repr__`.
fn format_overflow_policy(policy: &OverflowPolicy) -> String {
    match policy {
        OverflowPolicy::Drop => "OverflowPolicy.drop()".to_string(),
        OverflowPolicy::Block => "OverflowPolicy.block()".to_string(),
        OverflowPolicy::Timeout(duration) => {
            format!("OverflowPolicy.timeout({})", duration.as_millis())
        }
    }
}

/// Write overflow policy fields to a Python dictionary.
fn write_overflow_policy_fields(d: &Bound<'_, PyDict>, policy: &OverflowPolicy) -> PyResult<()> {
    match policy {
        OverflowPolicy::Drop => d.set_item("overflow_policy", "drop")?,
        OverflowPolicy::Block => d.set_item("overflow_policy", "block")?,
        OverflowPolicy::Timeout(duration) => {
            d.set_item("timeout_ms", duration.as_millis() as u64)?;
            d.set_item("overflow_policy", "timeout")?;
        }
    }
    Ok(())
}

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
        format_overflow_policy(&self.inner)
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
        // Try string identifier first
        if let Ok(py_str) = formatter.downcast::<PyString>() {
            self.set_formatter(py_str.to_str()?.to_owned());
            return Ok(());
        }

        // Then try callable-based formatter
        match crate::formatter::python::formatter_from_py(formatter) {
            Ok(instance) => {
                // The extracted formatter is already wrapped in a shared trait
                // object; storing it directly avoids double `Arc` wrapping via the
                // blanket `IntoFormatterConfig` implementation.
                self.formatter = Some(FormatterConfig::Instance(instance));
                Ok(())
            }
            Err(callable_err) => {
                let py = formatter.py();
                let callable_msg = callable_err
                    .value(py)
                    .repr()
                    .map(|r| r.to_string())
                    .unwrap_or_else(|_| "<unknown>".to_string());

                let msg = format!(
                    "invalid formatter: expected a string identifier or callable.\n\
                     - as callable: {callable_msg}",
                );
                Err(PyTypeError::new_err(msg))
            }
        }
    }

    /// Extend a Python dictionary with common builder fields.
    pub fn extend_py_dict(&self, d: &Bound<'_, PyDict>) -> PyResult<()> {
        if let Some(cap) = self.capacity {
            d.set_item("capacity", cap.get())?;
        }
        if let Some(ms) = self.flush_after_ms {
            d.set_item("flush_after_ms", ms.get())?;
        }
        if let Some(fmt) = &self.formatter {
            match fmt {
                FormatterConfig::Id(fid) => {
                    // Just a string id
                    d.set_item("formatter", fid.as_str())?;
                }
                FormatterConfig::Instance(_) => {
                    // Tagged object for non-serializable instance
                    let formatter_dict = PyDict::new(d.py());
                    formatter_dict.set_item("kind", "instance")?;
                    d.set_item("formatter", formatter_dict)?;
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
        if let Some(flush) = self.flush_after_records {
            d.set_item("flush_after_records", flush.get())?;
        }
        write_overflow_policy_fields(d, &self.overflow_policy)
    }
}
