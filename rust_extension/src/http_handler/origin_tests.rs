//! Admission tests for HTTP transport-origin suppression.
//!
//! A record produced by an HTTP worker must never enter an HTTP handler's
//! queue, or the delivery that produced it manufactures the next delivery.
//! These tests drive the real handler against a fake worker, so admission is
//! observed at the command channel rather than inferred from network effects.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crossbeam_channel::{Receiver, bounded};
use rstest::rstest;

use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::http_handler::FemtoHTTPHandler;
use crate::http_handler::queue::HTTPCommand;
use crate::level::FemtoLevel;
use crate::log_record::{FemtoLogRecord, RecordMetadata, RecordOrigin};
use crate::logger::FemtoLogger;
use crate::shutdown_signal::ShutdownSignal;
use crate::test_utils::collecting_handler::CollectingHandler;

/// Records a fake HTTP worker observed on its command channel.
type ObservedRecords = Arc<Mutex<Vec<FemtoLogRecord>>>;

/// Spawn a worker that records commands and acknowledges lifecycle requests.
///
/// The real worker owns the network; this one owns only the channel contract
/// these tests assert on. It acknowledges both `Flush` and `Shutdown` so the
/// handler's bounded waits complete deterministically.
fn spawn_fake_worker(
    rx: Receiver<HTTPCommand>,
) -> (ObservedRecords, Arc<ShutdownSignal>, thread::JoinHandle<()>) {
    let records = ObservedRecords::default();
    let observed = Arc::clone(&records);
    let signal = Arc::new(ShutdownSignal::new());
    let handle = thread::spawn(move || {
        for command in rx {
            match command {
                HTTPCommand::Record(record) => {
                    observed.lock().expect("fake worker mutex").push(record);
                }
                HTTPCommand::Flush(ack) => {
                    let _ = ack.send(());
                }
                HTTPCommand::Shutdown(ack) => {
                    let _ = ack.send(());
                    break;
                }
            }
        }
    });
    (records, signal, handle)
}

/// Build a handler around a fake worker and return the records it receives.
fn handler_with_fake_worker() -> (FemtoHTTPHandler, ObservedRecords) {
    let (tx, rx) = bounded(8);
    let (records, signal, handle) = spawn_fake_worker(rx);
    let handler =
        FemtoHTTPHandler::with_worker_for_test(tx, signal, handle, Duration::from_secs(1));
    (handler, records)
}

/// Build an information record with the requested origin.
fn origin_record(message: &str, origin: RecordOrigin) -> FemtoLogRecord {
    FemtoLogRecord::new("origin.http", FemtoLevel::Info, message).with_origin(origin)
}

/// A marked record is accepted and silently discarded, without enqueueing.
///
/// The drop accounting is asserted before the flush, because the flush clears
/// the pending figure: a mutant that counted the suppression as a drop would
/// otherwise look identical to the correct code.
#[rstest]
fn marked_records_are_suppressed_before_enqueueing() {
    let (handler, records) = handler_with_fake_worker();

    assert_eq!(
        handler.handle(origin_record(
            "delivery diagnostic",
            RecordOrigin::SelfDiagnostic
        )),
        Ok(())
    );
    assert_eq!(
        handler.warner.pending_drops(),
        0,
        "a suppressed transport diagnostic was never queued, so it is not a drop"
    );
    assert!(handler.flush(), "flush should be acknowledged");

    assert!(
        records.lock().expect("records mutex").is_empty(),
        "a transport diagnostic must not enter the queue"
    );
}

/// An unmarked record still takes the ordinary queue path.
#[rstest]
fn unmarked_records_are_enqueued() {
    let (handler, records) = handler_with_fake_worker();

    assert_eq!(
        handler.handle(origin_record(
            "application record",
            RecordOrigin::Application
        )),
        Ok(())
    );
    assert!(handler.flush(), "flush should be acknowledged");

    let recorded = records.lock().expect("records mutex");
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].message(), "application record");
}

/// Closing works normally after a marked record was suppressed.
#[rstest]
fn a_closed_handler_rejects_marked_records() {
    let (mut handler, records) = handler_with_fake_worker();
    handler.close();

    assert_eq!(
        handler.handle(origin_record(
            "late diagnostic",
            RecordOrigin::SelfDiagnostic
        )),
        Err(HandlerError::Closed)
    );
    assert!(records.lock().expect("records mutex").is_empty());
}

/// Marked records reach non-HTTP handlers but no HTTP handler's queue.
///
/// The logger dispatches to all three attachments. The collector represents
/// independent diagnostics, which must remain available; the two HTTP
/// handlers represent both the originating transport and any other endpoint
/// the same record reaches, so neither may enqueue it.
#[rstest]
fn marked_records_reach_only_the_non_http_handler() {
    let logger = FemtoLogger::new("origin.http.fanout".to_string()).expect("logger built");
    let (first, first_records) = handler_with_fake_worker();
    let (second, second_records) = handler_with_fake_worker();
    let first = Arc::new(first);
    let second = Arc::new(second);
    let collector = Arc::new(CollectingHandler::default());

    logger
        .add_handler(Arc::clone(&first) as Arc<dyn FemtoHandlerTrait>)
        .expect("first HTTP handler registers");
    logger
        .add_handler(Arc::clone(&second) as Arc<dyn FemtoHandlerTrait>)
        .expect("second HTTP handler registers");
    logger
        .add_handler(Arc::clone(&collector) as Arc<dyn FemtoHandlerTrait>)
        .expect("collector registers");

    logger.log_prebuilt(
        FemtoLogRecord::with_metadata(
            "origin.http.fanout",
            FemtoLevel::Info,
            "marked",
            RecordMetadata::default(),
        )
        .with_origin(RecordOrigin::SelfDiagnostic),
    );
    assert!(logger.flush_handlers(), "dispatch should drain");
    assert!(first.flush(), "first worker should acknowledge");
    assert!(second.flush(), "second worker should acknowledge");

    let collected = collector.collected();
    assert_eq!(collected.len(), 1, "the collector keeps the diagnostic");
    assert_eq!(collected[0].origin(), RecordOrigin::SelfDiagnostic);
    assert!(
        first_records.lock().expect("first records").is_empty(),
        "the originating handler must not enqueue its own diagnostic"
    );
    assert!(
        second_records.lock().expect("second records").is_empty(),
        "another HTTP handler must not continue the cycle"
    );

    logger.log_prebuilt(FemtoLogRecord::with_metadata(
        "origin.http.fanout",
        FemtoLevel::Info,
        "application",
        RecordMetadata::default(),
    ));
    assert!(logger.flush_handlers(), "dispatch should drain");
    assert!(first.flush(), "first worker should acknowledge");
    assert!(second.flush(), "second worker should acknowledge");

    assert_eq!(collector.collected().len(), 2);
    assert_eq!(first_records.lock().expect("first records").len(), 1);
    assert_eq!(second_records.lock().expect("second records").len(), 1);
}
