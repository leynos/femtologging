//! Assertions for bounded flush-barrier tracing events.

use std::any::Any;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Instant;

use super::FemtoLogger;
use super::InternalEventCollector;
use super::helpers::CapturedEvent;
use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::log_record::FemtoLogRecord;

/// A native handler whose flush fails so tracing can expose its failure stage.
struct UnconfirmedFlushHandler {
    /// Collector permitted to observe this worker's flush trace event.
    collector: InternalEventCollector,
}

impl FemtoHandlerTrait for UnconfirmedFlushHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn flush_until(&self, _deadline: Instant) -> bool {
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
