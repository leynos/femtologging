//! The core logger carries a record's origin without interpreting it.
//!
//! The producer path is adapter-free: it attaches no origin of its own and
//! rewrites none that a caller attached. These tests drive it with core types
//! only, so an adapter's vocabulary creeping back into the core record or the
//! producer would show up as a change in what a handler receives.

use std::sync::Arc;

use rstest::rstest;

use super::FemtoLogger;
use crate::handler::FemtoHandlerTrait;
use crate::level::FemtoLevel;
use crate::log_record::{FemtoLogRecord, RecordMetadata, RecordOrigin};
use crate::test_utils::collecting_handler::CollectingHandler;

/// A logger with one collector attached, as the core path sees it.
fn logger_with_collector() -> (FemtoLogger, Arc<CollectingHandler>) {
    let logger = FemtoLogger::new("producer.origin".to_string()).expect("logger built");
    let collector = Arc::new(CollectingHandler::default());
    logger
        .add_handler(Arc::clone(&collector) as Arc<dyn FemtoHandlerTrait>)
        .expect("the collector registers");
    (logger, collector)
}

/// The ordinary entry point delivers application records.
#[rstest]
fn the_ordinary_entry_point_delivers_application_records() {
    let (logger, collector) = logger_with_collector();

    logger.log_with_metadata(FemtoLevel::Info, "plain", RecordMetadata::default());
    assert!(logger.flush_handlers(), "dispatch should drain");

    let collected = collector.collected();
    assert_eq!(collected.len(), 1, "one record should reach the handler");
    assert_eq!(
        collected[0].origin(),
        RecordOrigin::Application,
        "the core path attaches no origin of its own"
    );
}

/// A caller-attached origin survives the producer path unchanged.
#[rstest]
fn a_prebuilt_record_keeps_the_origin_its_caller_attached() {
    let (logger, collector) = logger_with_collector();

    logger.log_prebuilt(
        FemtoLogRecord::with_metadata(
            "producer.origin",
            FemtoLevel::Info,
            "self report",
            RecordMetadata::default(),
        )
        .with_origin(RecordOrigin::SelfDiagnostic),
    );
    assert!(logger.flush_handlers(), "dispatch should drain");

    let collected = collector.collected();
    assert_eq!(collected.len(), 1, "one record should reach the handler");
    assert_eq!(
        collected[0].origin(),
        RecordOrigin::SelfDiagnostic,
        "the producer path must not rewrite the origin it was given"
    );
}
