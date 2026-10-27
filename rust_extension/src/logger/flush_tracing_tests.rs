//! Assertions for bounded flush-barrier tracing events.

use std::any::Any;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Instant;

use super::FemtoLogger;
use super::InternalEventCollector;
use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::log_record::FemtoLogRecord;

/// A native handler whose flush fails so tracing can expose its failure stage.
struct UnconfirmedFlushHandler;

impl FemtoHandlerTrait for UnconfirmedFlushHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn flush_until(&self, _deadline: Instant) -> bool {
        false
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Assert that a failed worker flush emits bounded operation and stage fields.
pub(super) fn assert_unconfirmed_flush_trace(collector: &InternalEventCollector) {
    collector.set_allow_all_threads(true);
    let logger = FemtoLogger::new("trace-flush-unconfirmed".to_owned())
        .expect("flush tracing logger should construct");
    logger
        .add_handler(Arc::new(UnconfirmedFlushHandler))
        .expect("flush tracing handler should attach");
    assert!(!logger.flush_handlers());
    collector.set_allow_all_threads(false);
    collector.stop_recording();

    let flush_events = collector
        .captured()
        .expect("flush worker event should be captured")
        .into_iter()
        .filter(|event| event.fields.get("operation").map(String::as_str) == Some("flush_handlers"))
        .collect::<Vec<_>>();
    let worker_event = flush_events
        .iter()
        .find(|event| event.fields.get("event").map(String::as_str) == Some("worker_complete"))
        .expect("the worker should trace its flush result");
    assert_eq!(worker_event.fields["outcome"], "unconfirmed");
    assert_eq!(
        worker_event.fields["failure_stage"],
        "handler_flush_unconfirmed"
    );
    assert!(worker_event.fields.contains_key("elapsed_us"));
    assert_eq!(
        worker_event
            .fields
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "elapsed_us",
            "event",
            "failure_stage",
            "operation",
            "outcome"
        ]),
        "flush events must not include logger names or record data: {:?}",
        worker_event.fields,
    );
}
