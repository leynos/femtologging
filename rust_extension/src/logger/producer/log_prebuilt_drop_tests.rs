//! The invalid-context drop path writes one diagnostic line per dropped record.
//!
//! `report_context_drop_tests` pins the bytes the writer helper emits, but a
//! helper test passes even when the logging path stops calling it. These tests
//! drive `log_prebuilt_to`, the function production reaches, with an injected
//! writer. That is the same code path `log_prebuilt` takes with stderr, so a
//! removed `report_context_drop` call fails here and nowhere else.

use std::collections::BTreeMap;
use std::sync::Arc;

use pyo3::prelude::*;

use super::FemtoLogger;
use crate::handler::FemtoHandlerTrait;
use crate::level::FemtoLevel;
use crate::log_record::{FemtoLogRecord, RecordMetadata};
use crate::manager;
use crate::test_utils::collecting_handler::CollectingHandler;

/// Build the metadata a rejected context payload needs.
///
/// `merge_context_values` refuses a value past its per-value bound, so one
/// oversize entry is enough to reach the drop arm without inventing a second
/// failure mode.
fn oversize_metadata() -> RecordMetadata {
    let mut key_values = BTreeMap::new();
    key_values.insert(String::from("oversize"), "x".repeat(1_025));
    RecordMetadata {
        key_values,
        ..Default::default()
    }
}

/// Fetch a logger of its own so a shared name cannot affect the count.
fn logger_named(py: Python<'_>, name: &str) -> Py<FemtoLogger> {
    manager::get_logger(py, name).expect("the test logger must be resolvable")
}

/// One dropped record writes exactly one diagnostic line.
///
/// The count is asserted alongside the text because the two are the same
/// event: the arm increments the counter and writes the line, so a test that
/// saw only one of them could pass with the other removed.
#[test]
fn one_dropped_record_writes_one_diagnostic_line() {
    Python::attach(|py| {
        let logger = logger_named(py, "log_prebuilt.drop.once");
        let handler = Arc::new(CollectingHandler::default()) as Arc<dyn FemtoHandlerTrait>;
        logger
            .borrow(py)
            .add_handler(handler.clone())
            .expect("handler");

        let record = FemtoLogRecord::with_metadata(
            "log_prebuilt.drop.once",
            FemtoLevel::Info,
            "dropped",
            oversize_metadata(),
        );

        let mut written = Vec::new();
        let result = logger.borrow(py).log_prebuilt_to(&mut written, record);

        assert!(result.is_none(), "an invalid context must drop the record");
        assert_eq!(
            logger.borrow(py).get_context_dropped(),
            1,
            "the dropped record must be counted",
        );
        assert!(
            handler
                .as_any()
                .downcast_ref::<CollectingHandler>()
                .expect("handler downcast")
                .collected()
                .is_empty(),
            "a dropped record must never reach a handler",
        );

        let text = String::from_utf8(written).expect("the diagnostic is UTF-8");
        assert_eq!(
            text.lines().count(),
            1,
            "one dropped record must write one line, got {text:?}",
        );
        assert!(
            text.contains("FemtoLogger: dropping record due to invalid context payload"),
            "the diagnostic must name the drop, got {text:?}",
        );
        assert!(
            text.ends_with('\n'),
            "the line must be newline terminated, got {text:?}",
        );
    });
}

/// Each dropped record adds exactly one line, and none is lost or repeated.
///
/// A single-record test cannot distinguish "one line per drop" from "one line
/// per call to the helper" or "the last write wins". Three records make the
/// difference visible: the count must exceed one, and every line must carry the
/// same diagnostic so a partial write cannot pass as a whole one.
#[test]
fn every_dropped_record_writes_its_own_line() {
    Python::attach(|py| {
        let logger = logger_named(py, "log_prebuilt.drop.thrice");

        let mut written = Vec::new();
        for _ in 0..3 {
            let record = FemtoLogRecord::with_metadata(
                "log_prebuilt.drop.thrice",
                FemtoLevel::Info,
                "dropped",
                oversize_metadata(),
            );
            assert!(
                logger
                    .borrow(py)
                    .log_prebuilt_to(&mut written, record)
                    .is_none(),
                "an invalid context must drop every record",
            );
        }

        assert_eq!(
            logger.borrow(py).get_context_dropped(),
            3,
            "every dropped record must be counted",
        );

        let text = String::from_utf8(written).expect("the diagnostics are UTF-8");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines.len(),
            3,
            "three dropped records must write three lines, got {text:?}",
        );
        for line in lines {
            assert!(
                line.contains("dropping record due to invalid context payload"),
                "every line must be the drop diagnostic, got {line:?}",
            );
        }
    });
}

/// A writer that refuses every write does not fail the logging call.
///
/// The diagnostic exists to explain a dropped record, so losing it must not
/// escalate into a second failure. The record is still counted and still
/// dropped, which is what a caller can observe.
#[test]
fn a_refusing_writer_does_not_fail_the_logging_call() {
    /// Refuse writes the way a closed descriptor does.
    struct RefusingWriter;

    impl std::io::Write for RefusingWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "diagnostic stream closed",
            ))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    Python::attach(|py| {
        let logger = logger_named(py, "log_prebuilt.drop.refused");
        let record = FemtoLogRecord::with_metadata(
            "log_prebuilt.drop.refused",
            FemtoLevel::Info,
            "dropped",
            oversize_metadata(),
        );

        // Returning normally is the assertion: a refused write must not panic
        // or propagate.
        let result = logger.borrow(py).log_prebuilt_to(RefusingWriter, record);

        assert!(result.is_none(), "the record must still be dropped");
        assert_eq!(
            logger.borrow(py).get_context_dropped(),
            1,
            "the drop must still be counted when its diagnostic cannot be written",
        );
    });
}
