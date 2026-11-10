//! Assertions for bounded flush-barrier tracing events.

use std::any::Any;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tracing::Subscriber;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;

use super::FemtoLogger;
use super::InternalEventCollector;
use super::helpers::{CapturedEvent, collect_fields};
use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;
use crate::logger::Shard;
use crate::sync::{Receiver, Sender, bounded};

/// A native handler whose flush fails so tracing can expose its failure stage.
struct UnconfirmedFlushHandler {
    /// Collector permitted to observe this worker's flush trace event.
    collector: InternalEventCollector,
}

impl FemtoHandlerTrait for UnconfirmedFlushHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn flush_until(&self, _context: &crate::handler::FlushContext) -> bool {
        // `flush_until` runs on the worker draining this flush, so permitting
        // the current thread captures this test's trace event only. A blanket
        // permission would also admit concurrent tests' workers, whose
        // `worker_complete` events are selected by the assertion below.
        self.collector
            .allow_current_thread()
            .expect("the flush worker should be permitted to emit its trace");
        false
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Whether `event` is a flush-barrier trace for the named stage.
fn is_flush_event(event: &CapturedEvent, stage: &str) -> bool {
    event.fields.get("operation").map(String::as_str) == Some("flush_handlers")
        && event.fields.get("event").map(String::as_str) == Some(stage)
}

/// Return the collected events that are not flush-barrier traces.
///
/// Helpers that dispatch records also flush the loggers they build, so their
/// barrier events would otherwise be counted against dispatch assertions.
pub(super) fn without_flush_events(events: &[CapturedEvent]) -> Vec<CapturedEvent> {
    events
        .iter()
        .filter(|event| event.fields.get("operation").map(String::as_str) != Some("flush_handlers"))
        .cloned()
        .collect()
}

/// Whether `event` is the worker's completion trace for a flush barrier.
fn is_flush_worker_complete(event: &CapturedEvent) -> bool {
    is_flush_event(event, "worker_complete")
}

/// Whether `event` is the caller's final barrier outcome.
fn is_flush_caller_complete(event: &CapturedEvent) -> bool {
    is_flush_event(event, "caller_complete")
}

/// Assert the exact bounded field set of one flush event.
fn assert_flush_event_fields(event: &CapturedEvent, expected: &[&str]) {
    assert_eq!(
        event
            .fields
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        expected.iter().copied().collect::<BTreeSet<_>>(),
        "flush events must not include logger names or record data: {:?}",
        event.fields,
    );
}

/// Assert that a failed worker flush emits bounded operation and stage fields.
pub(super) fn assert_unconfirmed_flush_trace(collector: &InternalEventCollector) {
    let logger = FemtoLogger::new("trace-flush-unconfirmed".to_owned())
        .expect("flush tracing logger should construct");
    logger
        .add_handler(Arc::new(UnconfirmedFlushHandler {
            collector: collector.clone(),
        }))
        .expect("flush tracing handler should attach");
    assert!(!logger.flush_handlers());
    // The worker traces its result before acknowledging, so a caller that
    // returned on the acknowledgement timeout can still observe the trace
    // afterwards. Wait for that event rather than stopping recording and
    // racing it.
    collector
        .wait_for_event(is_flush_worker_complete)
        .expect("the worker should trace its flush result");
    collector.stop_recording();

    let events = collector
        .captured()
        .expect("flush trace events should be captured");
    let worker_event = events
        .iter()
        .find(|event| is_flush_worker_complete(event))
        .expect("the worker should trace its flush result");
    assert_eq!(worker_event.fields["outcome"], "unconfirmed");
    assert_eq!(
        worker_event.fields["failure_stage"],
        "handler_flush_unconfirmed"
    );
    assert_flush_event_fields(
        worker_event,
        &[
            "elapsed_us",
            "event",
            "failure_stage",
            "operation",
            "outcome",
            "queue_wait_us",
        ],
    );

    // The caller's event is emitted after the worker's, inside the same span,
    // and reports the outcome the caller finally returned. The collector may
    // already hold caller events from earlier flushes in this test, so take
    // the most recent one.
    let caller_event = events
        .iter()
        .rev()
        .find(|event| is_flush_caller_complete(event))
        .expect("the caller should trace its final flush outcome");
    assert_eq!(caller_event.fields["outcome"], "unconfirmed");
    assert_eq!(
        caller_event.fields["failure_stage"],
        "handler_flush_unconfirmed"
    );
    assert_flush_event_fields(
        caller_event,
        &[
            "elapsed_us",
            "event",
            "failure_stage",
            "operation",
            "outcome",
        ],
    );
}

/// Handler that confirms its flush without buffering anything.
struct ConfirmingHandler;

impl FemtoHandlerTrait for ConfirmingHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn flush_until(&self, _context: &crate::handler::FlushContext) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Layer that captures every event delivered to its dispatcher.
///
/// Unlike [`InternalEventCollector`] this does not filter by thread: it is
/// installed only as a thread-local dispatcher, so the worker can reach it
/// only through the barrier's dispatcher hand-off. A captured worker event is
/// therefore proof of that hand-off rather than of a global default.
struct ThreadLocalCollector {
    /// Captured events retained for the assertion.
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

impl<S: Subscriber> Layer<S> for ThreadLocalCollector {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
        if event.metadata().target() != "femtologging::internal" {
            return;
        }
        if let Ok(mut events) = self.events.lock() {
            events.push(CapturedEvent {
                fields: collect_fields(event),
            });
        }
    }
}

/// A subscriber installed only on the calling thread must still see both the
/// caller's and the worker's events for the barrier it asked for.
#[test]
fn worker_events_reach_the_callers_thread_local_subscriber() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let dispatch =
        tracing::Dispatch::new(tracing_subscriber::registry().with(ThreadLocalCollector {
            events: Arc::clone(&events),
        }));
    let logger = FemtoLogger::new("thread-local-flush".to_owned())
        .expect("the thread-local flush logger should construct");
    logger
        .add_handler(Arc::new(ConfirmingHandler))
        .expect("the confirming handler should attach");

    let confirmed = tracing::dispatcher::with_default(&dispatch, || logger.flush_handlers());
    assert!(
        confirmed,
        "a handler that confirms should flush successfully"
    );

    let events = events
        .lock()
        .expect("the thread-local event list must not be poisoned");
    assert!(
        events.iter().any(is_flush_worker_complete),
        "the worker's event must reach the caller's thread-local subscriber: {events:?}"
    );
    assert!(
        events.iter().any(is_flush_caller_complete),
        "the caller's event must reach its own subscriber: {events:?}"
    );
}

/// Handler that parks its shard's worker until the test releases it.
struct StallingHandler {
    /// Signals that the worker has entered `handle`.
    started: Sender<()>,
    /// Releases the worker when a message arrives or the sender is dropped.
    release: Receiver<()>,
}

impl FemtoHandlerTrait for StallingHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        let _ = self.started.try_send(());
        let _ = self.release.recv_timeout(Duration::from_secs(5));
        Ok(())
    }

    fn flush_until(&self, _context: &crate::handler::FlushContext) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Wait until the first flush has inserted its marker into the shard queue.
fn wait_for_queued_marker(shard: &Shard) {
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while shard.queued_jobs_for_test() == 0 && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(
        shard.queued_jobs_for_test(),
        1,
        "the first flush marker should occupy the shallow shard queue"
    );
}

/// A marker refused by a full queue is traced on the caller side alone.
///
/// The worker is parked inside a handler and a first flush fills the shard's
/// only queue slot with its marker, so the second flush is refused at
/// admission. No worker ever handles that marker, which makes the caller's own
/// event the only trace an observer can rely on for the failure.
#[test]
fn enqueue_failure_is_traced_on_the_caller_side() {
    let shard = Box::leak(Box::new(
        Shard::start(0, 1).expect("the one-slot test shard should start"),
    ));
    let logger = FemtoLogger::assembled_for_test("trace-enqueue".to_owned(), shard);
    let (started, started_rx) = bounded(1);
    let (release, release_rx) = bounded(1);
    logger
        .add_handler(Arc::new(StallingHandler {
            started,
            release: release_rx,
        }))
        .expect("the stalling handler should attach");
    logger.log(FemtoLevel::Info, "park the worker");
    started_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the worker should enter the stalling handler");

    let events = Arc::new(Mutex::new(Vec::new()));
    let dispatch =
        tracing::Dispatch::new(tracing_subscriber::registry().with(ThreadLocalCollector {
            events: Arc::clone(&events),
        }));

    let confirmed = std::thread::scope(|scope| {
        let first_flush = scope.spawn(|| logger.flush_handlers());
        wait_for_queued_marker(shard);
        let refused = tracing::dispatcher::with_default(&dispatch, || logger.flush_handlers());
        let _ = release.send(());
        let _ = first_flush
            .join()
            .expect("the first flush thread should not panic");
        refused
    });

    assert!(
        !confirmed,
        "a flush whose marker cannot be enqueued must not report success"
    );

    let events = events
        .lock()
        .expect("the thread-local event list must not be poisoned");
    let flush_events = events
        .iter()
        .filter(|event| event.fields.get("operation").map(String::as_str) == Some("flush_handlers"))
        .collect::<Vec<_>>();
    assert_eq!(
        flush_events.len(),
        1,
        "the refused flush must leave exactly one caller event: {events:?}"
    );
    let caller_event = flush_events[0];
    assert_eq!(caller_event.fields["event"], "caller_complete");
    assert_eq!(caller_event.fields["outcome"], "unconfirmed");
    assert_eq!(
        caller_event.fields["failure_stage"],
        "marker_enqueue_timed_out"
    );
    assert_flush_event_fields(
        caller_event,
        &[
            "elapsed_us",
            "event",
            "failure_stage",
            "operation",
            "outcome",
        ],
    );
}
