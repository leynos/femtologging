//! Builder for [`FemtoFileHandler`].
//!
//! Provides a fluent API for configuring a file based logging handler.
//! Only a subset of options are currently supported; additional
//! parameters such as encoding and mode will be added as the project
//! evolves.

use pyo3::prelude::*;

use super::{file::*, HandlerBuildError, HandlerBuilderTrait};
use crate::{formatter::DefaultFormatter, handler::FemtoHandlerTrait};

/// Builder for constructing [`FemtoFileHandler`] instances.
#[pyclass]
#[derive(Clone, Debug)]
pub struct FileHandlerBuilder {
    path: String,
    capacity: Option<usize>,
    flush_interval: Option<usize>,
    overflow_policy: OverflowPolicy,
}

impl FileHandlerBuilder {
    /// Create a builder targeting the specified file path.
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            capacity: None,
            flush_interval: None,
            overflow_policy: OverflowPolicy::Drop,
        }
    }

    /// Set the bounded channel capacity.
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = Some(capacity);
        self
    }

    /// Set the periodic flush interval.
    pub fn with_flush_interval(mut self, interval: usize) -> Self {
        self.flush_interval = Some(interval);
        self
    }

    /// Set the overflow policy for the handler.
    pub fn with_overflow_policy(mut self, policy: OverflowPolicy) -> Self {
        self.overflow_policy = policy;
        self
    }

    fn validate(&self) -> Result<(), HandlerBuildError> {
        if let Some(cap) = self.capacity {
            if cap == 0 {
                return Err(HandlerBuildError::InvalidConfig(
                    "capacity must be greater than zero".to_string(),
                ));
            }
        }
        if let Some(flush) = self.flush_interval {
            if flush == 0 {
                return Err(HandlerBuildError::InvalidConfig(
                    "flush_interval must be greater than zero".to_string(),
                ));
            }
        }
        Ok(())
    }

    fn build_inner(&self) -> Result<FemtoFileHandler, HandlerBuildError> {
        self.validate()?;
        let mut cfg = HandlerConfig::default();
        if let Some(cap) = self.capacity {
            cfg.capacity = cap;
        }
        if let Some(flush) = self.flush_interval {
            cfg.flush_interval = flush;
        }
        cfg.overflow_policy = self.overflow_policy;
        let handler =
            FemtoFileHandler::with_capacity_flush_policy(&self.path, DefaultFormatter, cfg)?;
        Ok(handler)
    }
}

#[pymethods]
impl FileHandlerBuilder {
    #[new]
    fn py_new(path: String) -> Self {
        Self::new(path)
    }

    #[pyo3(name = "with_capacity")]
    fn py_with_capacity<'py>(mut slf: PyRefMut<'py, Self>, capacity: usize) -> PyRefMut<'py, Self> {
        slf.capacity = Some(capacity);
        slf
    }

    #[pyo3(name = "with_flush_interval")]
    fn py_with_flush_interval<'py>(
        mut slf: PyRefMut<'py, Self>,
        interval: usize,
    ) -> PyRefMut<'py, Self> {
        slf.flush_interval = Some(interval);
        slf
    }

    #[pyo3(name = "with_overflow_policy")]
    fn py_with_overflow_policy<'py>(
        mut slf: PyRefMut<'py, Self>,
        policy: &str,
        timeout_ms: Option<u64>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        slf.overflow_policy = match policy.to_ascii_lowercase().as_str() {
            "drop" => OverflowPolicy::Drop,
            "block" => OverflowPolicy::Block,
            "timeout" => {
                let ms = timeout_ms.ok_or_else(|| {
                    PyErr::new::<pyo3::exceptions::PyValueError, _>(
                        "timeout_ms required for timeout policy",
                    )
                })?;
                OverflowPolicy::Timeout(std::time::Duration::from_millis(ms))
            }
            other => {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "invalid overflow policy: {other}",
                )));
            }
        };
        Ok(slf)
    }

    /// Return a dictionary describing the builder configuration.
    fn as_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        use pyo3::types::PyDict;
        let d = PyDict::new(py);
        d.set_item("path", &self.path)?;
        if let Some(cap) = self.capacity {
            d.set_item("capacity", cap)?;
        }
        if let Some(flush) = self.flush_interval {
            d.set_item("flush_interval", flush)?;
        }
        let policy = match self.overflow_policy {
            OverflowPolicy::Drop => "drop",
            OverflowPolicy::Block => "block",
            OverflowPolicy::Timeout(_) => "timeout",
        };
        d.set_item("overflow_policy", policy)?;
        Ok(d.into())
    }

    /// Build the handler, raising ``ValueError`` or ``OSError`` on failure.
    fn build(&self) -> PyResult<FemtoFileHandler> {
        self.build_inner().map_err(PyErr::from)
    }
}

impl HandlerBuilderTrait for FileHandlerBuilder {
    fn build(&self) -> Result<Box<dyn FemtoHandlerTrait>, HandlerBuildError> {
        let handler = self.build_inner()?;
        Ok(Box::new(handler))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use tempfile::tempdir;

    #[rstest]
    fn build_file_handler() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.log");
        let builder = FileHandlerBuilder::new(path.to_string_lossy())
            .with_capacity(16)
            .with_flush_interval(1);
        let handler = builder.build_inner().unwrap();
        handler.flush();
    }

    #[rstest]
    fn reject_zero_capacity() {
        let builder = FileHandlerBuilder::new("log.txt").with_capacity(0);
        assert!(builder.build_inner().is_err());
    }
}
