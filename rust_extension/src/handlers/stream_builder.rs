//! Builder for [`FemtoStreamHandler`].
//!
//! Allows configuration of stream based handlers writing to `stdout` or
//! `stderr`. The builder exposes basic tuning for channel capacity and
//! flush timeout.

use std::time::Duration;

use pyo3::prelude::*;

use super::{HandlerBuildError, HandlerBuilderTrait};
use crate::{
    formatter::DefaultFormatter, handler::FemtoHandlerTrait, stream_handler::FemtoStreamHandler,
};

#[derive(Clone, Copy, Debug)]
enum StreamTarget {
    Stdout,
    Stderr,
}

/// Builder for constructing [`FemtoStreamHandler`] instances.
#[pyclass]
#[derive(Clone, Debug)]
pub struct StreamHandlerBuilder {
    target: StreamTarget,
    capacity: Option<usize>,
    flush_timeout_ms: Option<u64>,
}

impl StreamHandlerBuilder {
    /// Create a builder targeting `stdout`.
    pub fn stdout() -> Self {
        Self {
            target: StreamTarget::Stdout,
            capacity: None,
            flush_timeout_ms: None,
        }
    }

    /// Create a builder targeting `stderr`.
    pub fn stderr() -> Self {
        Self {
            target: StreamTarget::Stderr,
            capacity: None,
            flush_timeout_ms: None,
        }
    }

    /// Set the bounded channel capacity.
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = Some(capacity);
        self
    }

    /// Set the flush timeout in milliseconds.
    pub fn with_flush_timeout(mut self, timeout_ms: u64) -> Self {
        self.flush_timeout_ms = Some(timeout_ms);
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
        Ok(())
    }

    fn build_inner(&self) -> Result<FemtoStreamHandler, HandlerBuildError> {
        self.validate()?;
        let capacity = self.capacity.unwrap_or(1024);
        let timeout = Duration::from_millis(self.flush_timeout_ms.unwrap_or(1000));
        let handler = match self.target {
            StreamTarget::Stdout => FemtoStreamHandler::with_capacity_timeout(
                std::io::stdout(),
                DefaultFormatter,
                capacity,
                timeout,
            ),
            StreamTarget::Stderr => FemtoStreamHandler::with_capacity_timeout(
                std::io::stderr(),
                DefaultFormatter,
                capacity,
                timeout,
            ),
        };
        Ok(handler)
    }
}

#[pymethods]
impl StreamHandlerBuilder {
    #[new]
    fn py_new() -> Self {
        Self::stderr()
    }

    #[staticmethod]
    #[pyo3(name = "stdout")]
    fn py_stdout() -> Self {
        Self::stdout()
    }

    #[staticmethod]
    #[pyo3(name = "stderr")]
    fn py_stderr() -> Self {
        Self::stderr()
    }

    #[pyo3(name = "with_capacity")]
    fn py_with_capacity<'py>(mut slf: PyRefMut<'py, Self>, capacity: usize) -> PyRefMut<'py, Self> {
        slf.capacity = Some(capacity);
        slf
    }

    #[pyo3(name = "with_flush_timeout_ms")]
    fn py_with_flush_timeout<'py>(
        mut slf: PyRefMut<'py, Self>,
        timeout_ms: u64,
    ) -> PyRefMut<'py, Self> {
        slf.flush_timeout_ms = Some(timeout_ms);
        slf
    }

    /// Return a dictionary describing the builder configuration.
    fn as_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        use pyo3::types::PyDict;
        let d = PyDict::new(py);
        d.set_item(
            "target",
            match self.target {
                StreamTarget::Stdout => "stdout",
                StreamTarget::Stderr => "stderr",
            },
        )?;
        if let Some(cap) = self.capacity {
            d.set_item("capacity", cap)?;
        }
        if let Some(ms) = self.flush_timeout_ms {
            d.set_item("flush_timeout_ms", ms)?;
        }
        Ok(d.into())
    }

    /// Build the handler, raising ``ValueError`` on failure.
    fn build(&self) -> PyResult<FemtoStreamHandler> {
        self.build_inner().map_err(PyErr::from)
    }
}

impl HandlerBuilderTrait for StreamHandlerBuilder {
    fn build(&self) -> Result<Box<dyn FemtoHandlerTrait>, HandlerBuildError> {
        let handler = self.build_inner()?;
        Ok(Box::new(handler))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    fn build_stream_handler_stdout() {
        let builder = StreamHandlerBuilder::stdout().with_capacity(8);
        let handler = builder.build_inner().unwrap();
        handler.flush();
    }

    #[rstest]
    fn reject_zero_capacity() {
        let builder = StreamHandlerBuilder::stderr().with_capacity(0);
        assert!(builder.build_inner().is_err());
    }
}
