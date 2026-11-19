//! The core logger carries a record's origin without interpreting it.
//!
//! The producer path is adapter-free: it attaches no origin of its own and
//! rewrites none that a caller attached. These tests drive it with core types
//! only, so an adapter's vocabulary creeping back into the core record or the
//! producer would show up as a change in what a handler receives.

use std::sync::Arc;

use rstest::rstest;

use super::{FemtoLogger, HandlerAttachment};
use crate::filters::{FilterBuilderTrait, LevelFilterBuilder};
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

/// The handler-filter snapshot keeps the transport marker on the record.
///
/// A filtered attachment makes the producer capture a one-shot clone of the
/// record for its worker. That clone is what the HTTP handler eventually sees,
/// so it must keep the marker even though the filter may enrich it.
#[rstest]
fn handler_filter_snapshot_preserves_the_transport_origin() {
    let collector = Arc::new(CollectingHandler::default());
    let filter = LevelFilterBuilder::new()
        .with_max_level(FemtoLevel::Critical)
        .build()
        .expect("the accepting level filter should build");
    let attachment = HandlerAttachment {
        handler: Arc::clone(&collector) as Arc<dyn FemtoHandlerTrait>,
        filters: vec![filter],
    };
    let mut record = FemtoLogRecord::new(
        "origin.filter.snapshot",
        FemtoLevel::Info,
        "delivery diagnostic",
    )
    .with_origin(RecordOrigin::SelfDiagnostic);

    let selected = FemtoLogger::filter_handlers_for_test(&mut record, vec![attachment]);
    assert_eq!(
        selected.len(),
        1,
        "the accepting handler should be selected"
    );

    // The selected attachment is the snapshot wrapper. Its `handle` consumes
    // the saved clone instead of the unmarked fallback passed here.
    let fallback = FemtoLogRecord::new("origin.filter.fallback", FemtoLevel::Info, "fallback");
    selected[0]
        .handle(fallback)
        .expect("snapshot handler should accept the record");

    let collected = collector.collected();
    assert_eq!(collected.len(), 1);
    assert_eq!(
        collected[0].origin(),
        RecordOrigin::SelfDiagnostic,
        "the snapshot clone must keep the transport marker"
    );
}
