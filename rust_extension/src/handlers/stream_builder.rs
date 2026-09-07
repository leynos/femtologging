//! Builder for [`FemtoStreamHandler`].
//!
//! Allows configuration of stream based handlers writing to `stdout` or
//! `stderr`. The builder exposes basic tuning for channel capacity and
//! a millisecond-based flush threshold. `py_new` defaults to `stderr`
//! to mirror Python's `logging.StreamHandler`.

use std::{
    io::{self, Write},
    num::{NonZeroU64, NonZeroUsize},
    time::Duration,
};

#[cfg(feature = "python")]
use pyo3::prelude::*;

use super::{
    FormatterId,
    HandlerBuildError,
    HandlerBuilderTrait,
    common::{CommonBuilder, FormatterConfig, IntoFormatterConfig},
    file::DEFAULT_CHANNEL_CAPACITY,
};
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
    const fn as_str(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}

/// Builder for constructing [`FemtoStreamHandler`] instances.
#[cfg_attr(feature = "python", pyclass(from_py_object))]
#[derive(Clone, Debug)]
pub struct StreamHandlerBuilder {
    target: StreamTarget,
    common: CommonBuilder,
}

impl StreamHandlerBuilder {
    /// Create a builder targeting `stdout`.
    #[must_use]
    pub fn stdout() -> Self {
        Self {
            target: StreamTarget::Stdout,
            common: CommonBuilder::default(),
        }
    }

    /// Create a builder targeting `stderr`.
    #[must_use]
    pub fn stderr() -> Self {
        Self {
            target: StreamTarget::Stderr,
            common: CommonBuilder::default(),
        }
    }

    /// Attach a formatter instance or identifier.
    #[must_use]
    pub fn with_formatter<F>(mut self, formatter: F) -> Self
    where
        F: IntoFormatterConfig,
    {
        self.common.set_formatter(formatter);
        self
    }

    fn is_capacity_valid(&self) -> Result<(), HandlerBuildError> { self.common.is_capacity_valid() }

    fn is_flush_after_ms_valid(&self) -> Result<(), HandlerBuildError> {
        CommonBuilder::ensure_non_zero(
            "flush_after_ms",
            self.common.flush_after_ms.map(NonZeroU64::get),
        )
    }

    fn resolved_capacity(&self) -> usize {
        self.common
            .capacity
            .map_or(DEFAULT_CHANNEL_CAPACITY, NonZeroUsize::get)
    }

    fn resolved_flush_after(&self) -> Duration {
        Duration::from_millis(
            self.common
                .flush_after_ms
                .map_or(CommonBuilder::DEFAULT_FLUSH_AFTER_MS, NonZeroU64::get),
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
        let flush_after = self.resolved_flush_after();
        FemtoStreamHandler::with_capacity_timeout(writer, formatter, capacity, flush_after)
    }

    fn validate(&self) -> Result<(), HandlerBuildError> {
        self.is_capacity_valid()?;
        self.is_flush_after_ms_valid()?;
        Ok(())
    }
}

#[path = "stream_builder_python_bindings.rs"]
mod python_bindings;

#[cfg(feature = "python")]
impl AsPyDict for StreamHandlerBuilder {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
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
    //! Tests for the stream handler builder.

    use std::{
        io::{self, Write},
        sync::{Arc, Mutex},
    };

    #[cfg(feature = "python")]
    use pyo3::Python;
    use rstest::rstest;

    use super::{super::test_helpers::assert_build_err, *};
    use crate::{
        formatter::FemtoFormatter,
        handler::FemtoHandlerTrait,
        log_record::FemtoLogRecord,
    };

    #[derive(Clone, Copy, Debug)]
    struct UpperFormatter;

    impl FemtoFormatter for UpperFormatter {
        fn format(&self, record: &FemtoLogRecord) -> String { record.message().to_uppercase() }
    }

    #[derive(Clone, Debug, Default)]
    struct TestWriter {
        buffer: Arc<Mutex<Vec<u8>>>,
    }

    impl TestWriter {
        fn new(buffer: Arc<Mutex<Vec<u8>>>) -> Self { Self { buffer } }
    }

    impl Write for TestWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            // Recover from poisoning: the buffer contents remain valid data.
            let mut guard = self
                .buffer
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }

    #[rstest]
    #[case(StreamHandlerBuilder::stdout())]
    #[case(StreamHandlerBuilder::stderr())]
    fn build_stream_handler_with_capacity(#[case] builder: StreamHandlerBuilder) {
        let configured_builder = builder.with_capacity(8);
        let mut handler = configured_builder
            .build_inner()
            .expect("build_inner must succeed for a valid builder");
        handler.flush();
        handler.close();
    }

    #[rstest]
    #[case(StreamHandlerBuilder::stdout())]
    #[case(StreamHandlerBuilder::stderr())]
    fn build_stream_handler_with_custom_formatter(#[case] builder: StreamHandlerBuilder) {
        let configured_builder = builder.with_formatter(UpperFormatter).with_capacity(4);
        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut handler = match configured_builder.common.formatter.as_ref() {
            Some(FormatterConfig::Instance(fmt)) => configured_builder
                .build_with_writer(TestWriter::new(Arc::clone(&sink)), fmt.clone_arc()),
            Some(FormatterConfig::Id(FormatterId::Default)) | None => configured_builder
                .build_with_writer(TestWriter::new(Arc::clone(&sink)), DefaultFormatter),
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
        let configured_builder = builder.with_capacity(0);
        assert_build_err(
            &configured_builder,
            "build_inner must fail for zero capacity",
        );
    }

    #[cfg(feature = "python")]
    #[test]
    fn python_rejects_zero_flush_after_ms() {
        use pyo3::types::PyAnyMethods;

        Python::attach(|py| {
            let builder = pyo3::Py::new(py, StreamHandlerBuilder::stderr())
                .expect("Py::new must create a stream builder");
            let err = builder
                .bind(py)
                .call_method1("with_flush_after_ms", (0,))
                .expect_err("with_flush_after_ms must reject zero");
            assert!(err.is_instance_of::<pyo3::exceptions::PyValueError>(py));
        });
    }

    #[rstest]
    #[case(StreamHandlerBuilder::stdout())]
    #[case(StreamHandlerBuilder::stderr())]
    fn reject_unknown_formatter(#[case] builder: StreamHandlerBuilder) {
        let configured_builder = builder.with_formatter("does-not-exist");
        assert_build_err(
            &configured_builder,
            "build_inner must fail for unknown formatter",
        );
    }
}
