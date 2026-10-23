//! Tests for the stream handler builder.

use super::super::test_helpers::assert_build_err;
use super::*;
#[cfg(feature = "python")]
use pyo3::Python;
use rstest::rstest;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use crate::{formatter::FemtoFormatter, handler::FemtoHandlerTrait, log_record::FemtoLogRecord};

/// Formatter fixture used to distinguish custom stream formatting.
#[derive(Clone, Copy, Debug)]
struct UpperFormatter;

impl FemtoFormatter for UpperFormatter {
    fn format(&self, record: &FemtoLogRecord) -> String {
        record.message().to_uppercase()
    }
}

/// In-memory writer used to inspect bytes emitted by the stream worker.
#[derive(Clone, Debug, Default)]
struct TestWriter {
    /// Bytes written by the handler's worker thread.
    buffer: Arc<Mutex<Vec<u8>>>,
}

impl TestWriter {
    fn new(buffer: Arc<Mutex<Vec<u8>>>) -> Self {
        Self { buffer }
    }
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
    let builder = builder.with_formatter("does-not-exist");
    assert_build_err(&builder, "build_inner must fail for unknown formatter");
}
