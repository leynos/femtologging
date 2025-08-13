//! Builder for [`FemtoStreamHandler`].
//!
//! Allows configuration of stream based handlers writing to `stdout` or
//! `stderr`. The builder exposes basic tuning for channel capacity and
//! a millisecond-based flush timeout. `py_new` defaults to `stderr`
//! to mirror Python's `logging.StreamHandler`.

use std::{num::NonZeroUsize, time::Duration};

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
    formatter_id: Option<String>,
    flush_timeout_ms: Option<u64>,
}

impl StreamHandlerBuilder {
    /// Create a builder targeting `stdout`.
    pub fn stdout() -> Self {
        Self {
            target: StreamTarget::Stdout,
            common: CommonBuilder::default(),
            formatter_id: None,
            flush_timeout_ms: None,
        }
    }

    /// Create a builder targeting `stderr`.
    pub fn stderr() -> Self {
        Self {
            target: StreamTarget::Stderr,
            common: CommonBuilder::default(),
            formatter_id: None,
            flush_timeout_ms: None,
        }
    }

    /// Set the bounded channel capacity.
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.common.capacity = NonZeroUsize::new(capacity);
        self.common.capacity_set = true;
        self
    }

    /// Set the flush timeout in milliseconds. Must be greater than zero.
    pub fn with_flush_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.flush_timeout_ms = Some(timeout_ms);
        self
    }

    /// Set the formatter identifier.
    pub fn with_formatter(mut self, formatter_id: impl Into<String>) -> Self {
        self.formatter_id = Some(formatter_id.into());
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
    /// Create a new `StreamHandlerBuilder` defaulting to `stderr`.
    ///
    /// Mirrors Python's `logging.StreamHandler` default stream.
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
        slf.common.capacity = NonZeroUsize::new(capacity);
        slf.common.capacity_set = true;
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

    #[pyo3(name = "with_formatter")]
    fn py_with_formatter<'py>(
        mut slf: PyRefMut<'py, Self>,
        formatter_id: String,
    ) -> PyRefMut<'py, Self> {
        slf.formatter_id = Some(formatter_id);
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
            d.set_item("capacity", cap.get())?;
        }
        if let Some(ms) = self.flush_timeout_ms {
            d.set_item("flush_timeout_ms", ms)?;
        }
        if let Some(fid) = &self.formatter_id {
            d.set_item("formatter_id", fid)?;
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
        let capacity = self.common.capacity.map(|c| c.get()).unwrap_or(1024);
        let timeout = Duration::from_millis(self.flush_timeout_ms.unwrap_or(1000));
        let formatter = match self.formatter_id.as_deref() {
            Some("default") | None => DefaultFormatter,
            Some(other) => {
                return Err(HandlerBuildError::InvalidConfig(format!(
                    "unknown formatter id: {other}",
                )))
            }
        };
        let handler = match self.target {
            StreamTarget::Stdout => FemtoStreamHandler::with_capacity_timeout(
                std::io::stdout(),
                formatter,
                capacity,
                timeout,
            ),
            StreamTarget::Stderr => FemtoStreamHandler::with_capacity_timeout(
                std::io::stderr(),
                formatter,
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
    #[case(StreamHandlerBuilder::stdout())]
    #[case(StreamHandlerBuilder::stderr())]
    fn build_stream_handler_with_capacity(#[case] builder: StreamHandlerBuilder) {
        let builder = builder.with_capacity(8);
        let mut handler = builder
            .build_inner()
            .expect("build_inner must succeed for a valid builder");
        handler.flush();
        handler.close();
    }

    #[rstest]
    #[case(StreamHandlerBuilder::stdout())]
    #[case(StreamHandlerBuilder::stderr())]
    fn reject_zero_capacity(#[case] builder: StreamHandlerBuilder) {
        let builder = builder.with_capacity(0);
        assert_build_err(&builder, "build_inner must fail for zero capacity");
    }

    #[rstest]
    #[case(StreamHandlerBuilder::stdout())]
    #[case(StreamHandlerBuilder::stderr())]
    fn reject_zero_flush_timeout(#[case] builder: StreamHandlerBuilder) {
        let builder = builder.with_flush_timeout_ms(0);
        assert_build_err(&builder, "build_inner must fail for zero flush timeout");
    }

    #[rstest]
    #[case(StreamHandlerBuilder::stdout())]
    #[case(StreamHandlerBuilder::stderr())]
    fn reject_unknown_formatter(#[case] builder: StreamHandlerBuilder) {
        let builder = builder.with_formatter("does-not-exist");
        assert_build_err(&builder, "build_inner must fail for unknown formatter");
    }
}
