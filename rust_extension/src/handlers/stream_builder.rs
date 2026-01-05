//! Builder for [`FemtoStreamHandler`].
//!
//! Allows configuration of stream based handlers writing to `stdout` or
//! `stderr`. The builder exposes basic tuning for channel capacity and
//! a millisecond-based flush timeout. `py_new` defaults to `stderr`
//! to mirror Python's `logging.StreamHandler`.

use std::{
    io::{self, Write},
    num::NonZeroU64,
    time::Duration,
};

#[cfg(feature = "python")]
use pyo3::prelude::*;

use super::{
    FormatterId, HandlerBuildError, HandlerBuilderTrait,
    common::{CommonBuilder, FormatterConfig, IntoFormatterConfig},
};

use crate::handlers::builder_macros::builder_methods;
#[cfg(test)]
use crate::level::FemtoLevel;
#[cfg(feature = "python")]
use crate::macros::{AsPyDict, dict_into_py};
use crate::{
    formatter::{DefaultFormatter, FemtoFormatter},
    stream_handler::FemtoStreamHandler,
};

#[derive(Clone, Copy, Debug)]
enum StreamTarget {
    Stdout,
    Stderr,
}

impl StreamTarget {
    #[cfg(feature = "python")]
    fn as_str(&self) -> &'static str {
        match self {
            StreamTarget::Stdout => "stdout",
            StreamTarget::Stderr => "stderr",
        }
    }
}

/// Builder for constructing [`FemtoStreamHandler`] instances.
#[cfg_attr(feature = "python", pyclass)]
#[derive(Clone, Debug)]
pub struct StreamHandlerBuilder {
    target: StreamTarget,
    common: CommonBuilder,
}

impl StreamHandlerBuilder {
    /// Create a builder targeting `stdout`.
    pub fn stdout() -> Self {
        Self {
            target: StreamTarget::Stdout,
            common: CommonBuilder::default(),
        }
    }

    /// Create a builder targeting `stderr`.
    pub fn stderr() -> Self {
        Self {
            target: StreamTarget::Stderr,
            common: CommonBuilder::default(),
        }
    }

    /// Attach a formatter instance or identifier.
    pub fn with_formatter<F>(mut self, formatter: F) -> Self
    where
        F: IntoFormatterConfig,
    {
        self.common.set_formatter(formatter);
        self
    }

    fn is_capacity_valid(&self) -> Result<(), HandlerBuildError> {
        self.common.is_capacity_valid()
    }

    fn is_flush_timeout_valid(&self) -> Result<(), HandlerBuildError> {
        CommonBuilder::ensure_non_zero(
            "flush_timeout_ms",
            self.common.flush_timeout_ms.map(NonZeroU64::get),
        )
    }

    fn resolved_capacity(&self) -> usize {
        self.common.capacity.map(|c| c.get()).unwrap_or(1024)
    }

    fn resolved_flush_timeout(&self) -> Duration {
        Duration::from_millis(
            self.common
                .flush_timeout_ms
                .map(NonZeroU64::get)
                .unwrap_or(CommonBuilder::DEFAULT_FLUSH_TIMEOUT_MS),
        )
    }

    fn build_with_formatter<F>(&self, formatter: F) -> FemtoStreamHandler
    where
        F: FemtoFormatter + Send + 'static,
    {
        match self.target {
            StreamTarget::Stdout => self.build_with_writer(io::stdout(), formatter),
            StreamTarget::Stderr => self.build_with_writer(io::stderr(), formatter),
        }
    }

    fn build_with_writer<W, F>(&self, writer: W, formatter: F) -> FemtoStreamHandler
    where
        W: Write + Send + 'static,
        F: FemtoFormatter + Send + 'static,
    {
        let capacity = self.resolved_capacity();
        let timeout = self.resolved_flush_timeout();
        FemtoStreamHandler::with_capacity_timeout(writer, formatter, capacity, timeout)
    }

    fn validate(&self) -> Result<(), HandlerBuildError> {
        self.is_capacity_valid()?;
        self.is_flush_timeout_valid()?;
        Ok(())
    }
}

builder_methods! {
    impl StreamHandlerBuilder {
        capacity {
            self_ident = builder,
            setter = |builder_ref, capacity| {
                builder_ref.common.set_capacity(capacity);
            }
        };
        methods {
            method {
                doc: "Set the flush timeout in milliseconds.\n\n# Validation\n\nAccepts a `NonZeroU64` so both Rust and Python callers must provide a timeout greater than zero.",
                rust_name: with_flush_timeout_ms,
                py_fn: py_with_flush_timeout_ms,
                py_name: "with_flush_timeout_ms",
                py_text_signature: "(self, timeout_ms)",
                rust_args: (timeout_ms: NonZeroU64),
                py_args: (timeout_ms: u64),
                py_prelude: {
                    let timeout_ms = NonZeroU64::new(timeout_ms).ok_or_else(|| {
                        pyo3::exceptions::PyValueError::new_err(
                            "flush_timeout_ms must be greater than zero",
                        )
                    })?;
                },
                self_ident: builder,
                body: {
                    builder.common.flush_timeout_ms = Some(timeout_ms);
                }
            }
        }
        extra_py_methods {
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
            fn as_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
                self.as_pydict(py)
            }

            /// Build the handler, raising ``HandlerConfigError`` or ``HandlerIOError`` on
            /// failure.
            fn build(&self) -> PyResult<FemtoStreamHandler> {
                <Self as HandlerBuilderTrait>::build_inner(self).map_err(PyErr::from)
            }
        }
    }
}

#[cfg(feature = "python")]
impl AsPyDict for StreamHandlerBuilder {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<PyObject> {
        use pyo3::types::PyDict;
        let d = PyDict::new(py);
        d.set_item("target", self.target.as_str())?;
        self.common.extend_py_dict(&d)?;
        dict_into_py(d, py)
    }
}

impl HandlerBuilderTrait for StreamHandlerBuilder {
    type Handler = FemtoStreamHandler;

    fn build_inner(&self) -> Result<Self::Handler, HandlerBuildError> {
        self.validate()?;
        let handler = match self.common.formatter.as_ref() {
            Some(FormatterConfig::Instance(fmt)) => self.build_with_formatter(fmt.clone_arc()),
            Some(FormatterConfig::Id(FormatterId::Default)) | None => {
                self.build_with_formatter(DefaultFormatter)
            }
            Some(FormatterConfig::Id(FormatterId::Custom(other))) => {
                return Err(HandlerBuildError::InvalidConfig(format!(
                    "unknown formatter id: {other}",
                )));
            }
        };
        Ok(handler)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::assert_build_err;
    use super::*;
    #[cfg(feature = "python")]
    use pyo3::Python;
    use rstest::rstest;
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};

    use crate::{
        formatter::FemtoFormatter, handler::FemtoHandlerTrait, log_record::FemtoLogRecord,
    };

    #[derive(Clone, Copy, Debug)]
    struct UpperFormatter;

    impl FemtoFormatter for UpperFormatter {
        fn format(&self, record: &FemtoLogRecord) -> String {
            record.message.to_uppercase()
        }
    }

    #[derive(Clone, Debug, Default)]
    struct TestWriter {
        buffer: Arc<Mutex<Vec<u8>>>,
    }

    impl TestWriter {
        fn new(buffer: Arc<Mutex<Vec<u8>>>) -> Self {
            Self { buffer }
        }
    }

    impl Write for TestWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let mut guard = self
                .buffer
                .lock()
                .expect("buffer mutex must not be poisoned");
            guard.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

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
    fn build_stream_handler_with_custom_formatter(#[case] builder: StreamHandlerBuilder) {
        let builder = builder.with_formatter(UpperFormatter).with_capacity(4);
        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut handler = match builder.common.formatter.as_ref() {
            Some(FormatterConfig::Instance(fmt)) => {
                builder.build_with_writer(TestWriter::new(Arc::clone(&sink)), fmt.clone_arc())
            }
            Some(FormatterConfig::Id(FormatterId::Default)) | None => {
                builder.build_with_writer(TestWriter::new(Arc::clone(&sink)), DefaultFormatter)
            }
            Some(FormatterConfig::Id(FormatterId::Custom(other))) => {
                panic!("unexpected custom formatter id: {other}")
            }
        };
        FemtoHandlerTrait::handle(
            &handler,
            FemtoLogRecord::new("logger", FemtoLevel::Info, "stream hello"),
        )
        .expect("custom formatter stream write must succeed");
        assert!(handler.flush(), "flush must succeed for stream handler");
        handler.close();
        let output = String::from_utf8(
            sink.lock()
                .expect("buffer mutex must not be poisoned")
                .clone(),
        )
        .expect("stream output must be valid UTF-8");
        assert!(
            output.contains("STREAM HELLO"),
            "custom formatter must uppercase the stream payload: {output:?}",
        );
    }

    #[rstest]
    #[case(StreamHandlerBuilder::stdout())]
    #[case(StreamHandlerBuilder::stderr())]
    fn reject_zero_capacity(#[case] builder: StreamHandlerBuilder) {
        let builder = builder.with_capacity(0);
        assert_build_err(&builder, "build_inner must fail for zero capacity");
    }

    #[cfg(feature = "python")]
    #[test]
    fn python_rejects_zero_flush_timeout() {
        use pyo3::types::PyAnyMethods;

        Python::with_gil(|py| {
            let builder = pyo3::Py::new(py, StreamHandlerBuilder::stderr())
                .expect("Py::new must create a stream builder");
            let err = builder
                .bind(py)
                .call_method1("with_flush_timeout_ms", (0,))
                .expect_err("with_flush_timeout_ms must reject zero");
            assert!(err.is_instance_of::<pyo3::exceptions::PyValueError>(py));
        });
    }

    #[rstest]
    #[case(StreamHandlerBuilder::stdout())]
    #[case(StreamHandlerBuilder::stderr())]
    fn reject_unknown_formatter(#[case] builder: StreamHandlerBuilder) {
        let builder = builder.with_formatter("does-not-exist");
        assert_build_err(&builder, "build_inner must fail for unknown formatter");
    }
}
