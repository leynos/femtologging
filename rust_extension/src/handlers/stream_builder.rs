//! Builder for [`FemtoStreamHandler`].
//!
//! Allows configuration of stream based handlers writing to `stdout` or
//! `stderr`. The builder exposes basic tuning for channel capacity and
//! flush timeout.

use std::time::Duration;

use pyo3::prelude::*;

use super::{common::CommonBuilder, HandlerBuildError, HandlerBuilderTrait};
use crate::{formatter::DefaultFormatter, stream_handler::FemtoStreamHandler};

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
    common: CommonBuilder,
    flush_timeout_ms: Option<u64>,
}

impl StreamHandlerBuilder {
    /// Create a builder targeting `stdout`.
    pub fn stdout() -> Self {
        Self {
            target: StreamTarget::Stdout,
            common: CommonBuilder::default(),
            flush_timeout_ms: None,
        }
    }

    /// Create a builder targeting `stderr`.
    pub fn stderr() -> Self {
        Self {
            target: StreamTarget::Stderr,
            common: CommonBuilder::default(),
            flush_timeout_ms: None,
        }
    }

    /// Set the bounded channel capacity.
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.common.capacity = Some(capacity);
        self
    }

    /// Set the flush timeout in milliseconds.
    pub fn with_flush_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.flush_timeout_ms = Some(timeout_ms);
        self
    }

    fn is_capacity_valid(&self) -> Result<(), HandlerBuildError> {
        self.common.is_capacity_valid()
    }

    fn is_flush_timeout_valid(&self) -> Result<(), HandlerBuildError> {
        CommonBuilder::ensure_non_zero("flush_timeout_ms", self.flush_timeout_ms)
    }

    fn validate(&self) -> Result<(), HandlerBuildError> {
        self.is_capacity_valid()?;
        self.is_flush_timeout_valid()?;
        Ok(())
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
        slf.common.capacity = Some(capacity);
        slf
    }

    #[pyo3(name = "with_flush_timeout_ms")]
    fn py_with_flush_timeout_ms<'py>(
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
        if let Some(cap) = self.common.capacity {
            d.set_item("capacity", cap)?;
        }
        if let Some(ms) = self.flush_timeout_ms {
            d.set_item("flush_timeout_ms", ms)?;
        }
        Ok(d.into())
    }

    /// Build the handler, raising ``HandlerConfigError`` or ``HandlerIOError`` on
    /// failure.
    fn build(&self) -> PyResult<FemtoStreamHandler> {
        <Self as HandlerBuilderTrait>::build_inner(self).map_err(PyErr::from)
    }
}

impl HandlerBuilderTrait for StreamHandlerBuilder {
    type Handler = FemtoStreamHandler;

    fn build_inner(&self) -> Result<Self::Handler, HandlerBuildError> {
        self.validate()?;
        let capacity = self.common.capacity.unwrap_or(1024);
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

#[cfg(test)]
mod tests {
    use super::super::test_helpers::assert_build_err;
    use super::*;
    use rstest::rstest;

    #[rstest]
    fn build_stream_handler_stdout() {
        let builder = StreamHandlerBuilder::stdout().with_capacity(8);
        let handler = builder
            .build_inner()
            .expect("build_inner must succeed for a valid stdout builder");
        handler.flush();
    }

    #[rstest]
    fn reject_zero_capacity() {
        let builder = StreamHandlerBuilder::stderr().with_capacity(0);
        assert_build_err(&builder, "build_inner must fail for zero capacity");
    }

    #[rstest]
    fn reject_zero_flush_timeout() {
        let builder = StreamHandlerBuilder::stdout().with_flush_timeout_ms(0);
        assert_build_err(&builder, "build_inner must fail for zero flush timeout");
    }
}
