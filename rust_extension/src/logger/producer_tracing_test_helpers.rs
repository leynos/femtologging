//! Shared field collection and assertions for producer tracing tests.
//!
//! The helpers here back `producer_tracing_tests`. `CapturedEvent`,
//! `EventFieldCollector` and `collect_fields` turn a `tracing` event into
//! inspectable fields; `ExpectedEvent`, `EXPECTED_EVENTS` and
//! `EXPECTED_FILTER_OUTCOMES` describe the outcomes that `assert_event`,
//! `assert_events` and `assert_filter_events` compare. The `emit_*` family
//! includes `emit_python_handler_dispatch`, `emit_capture_failure` and
//! `emit_rejected_native_dispatch`, which drive `FemtoLogger` per outcome.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use pyo3::prelude::*;
use tracing::field::{Field, Visit};

use super::{
    ContextVarsSnapshotProvider, FemtoLogger, FixedDecisionFilter, InternalEventCollector,
    TestSnapshotProvider, python_logger, with_sensitive_context,
};
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;
use crate::manager;
use crate::test_utils::collecting_handler::CollectingHandler;

#[path = "producer_tracing_test_rejection.rs"]
mod rejection;

pub(super) fn emit_rejected_native_dispatch() -> Result<(), String> {
    rejection::emit_rejected_native_dispatch()
}

/// One tracing event captured by the test subscriber.
#[derive(Clone, Debug)]
pub(super) struct CapturedEvent {
    /// Event field names mapped to their rendered values.
    pub(super) fields: BTreeMap<String, String>,
}

/// Expected identifying fields and occurrence count for a tracing event.
#[derive(Clone, Copy, Debug)]
pub(super) struct ExpectedEvent {
    /// Operation name identifying the producer or worker action.
    pub(super) operation: &'static str,
    /// Handler category associated with the event.
    pub(super) handler_kind: &'static str,
    /// Field that records the operation outcome.
    pub(super) outcome_field: &'static str,
    /// Expected outcome value for the event.
    pub(super) outcome: &'static str,
    /// Number of matching events expected in the scenario.
    pub(super) count: usize,
}

// The expected event tables live in a sibling data file. `include!` keeps the
// compile-time array types the tracing tests and helpers below rely on.
include!("producer_tracing_expected_tables.rs");

/// Tracing visitor that stores each recorded field's rendered value.
#[derive(Default)]
struct EventFieldCollector {
    /// Collected event values, indexed by field name.
    fields: BTreeMap<String, String>,
}

impl Visit for EventFieldCollector {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.fields
            .insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields
            .insert(field.name().to_owned(), value.to_owned());
    }
}

pub(super) fn collect_fields(event: &tracing::Event<'_>) -> BTreeMap<String, String> {
    let mut visitor = EventFieldCollector::default();
    event.record(&mut visitor);
    visitor.fields
}

fn python_dispatch_logger(
    collector: &InternalEventCollector,
    handler_fails: bool,
) -> Result<FemtoLogger, String> {
    python_logger(
        Arc::new(ContextVarsSnapshotProvider),
        collector.clone(),
        handler_fails,
    )
    .map_err(|error| error.to_string())
}

/// Result simulated by the Python-backed test handler.
#[derive(Clone, Copy)]
pub(super) enum PythonHandlerOutcome {
    /// Handler dispatch completes successfully.
    Success,
    /// Handler dispatch returns an error.
    Failure,
}

pub(super) fn emit_python_handler_dispatch(
    collector: &InternalEventCollector,
    outcome: PythonHandlerOutcome,
) -> Result<(), String> {
    let (handler_fails, expected_events, description) = match outcome {
        PythonHandlerOutcome::Success => (false, 3, "successful"),
        PythonHandlerOutcome::Failure => (true, 3, "failing"),
    };
    let expected_event_count = collector.captured()?.len().saturating_add(expected_events);
    let logger = python_dispatch_logger(collector, handler_fails)?;
    let mut record_enqueued = false;
    with_sensitive_context(|| {
        record_enqueued = logger
            .log(FemtoLevel::Info, "message-secret-not-traced")
            .is_some();
    })
    .map_err(|error| error.to_string())?;
    if !record_enqueued {
        return Err(format!(
            "{description} handler dispatch should enqueue its record"
        ));
    }
    collector.wait_for_event_count(expected_event_count)
}

pub(super) fn emit_capture_failure(collector: &InternalEventCollector) -> Result<(), String> {
    let logger = python_logger(
        Arc::new(TestSnapshotProvider { fail: true }),
        collector.clone(),
        false,
    )
    .map_err(|error| error.to_string())?;
    if logger
        .log(FemtoLevel::Info, "message-secret-not-traced")
        .is_none()
    {
        return Err("capture failure should still accept its record".to_owned());
    }
    drop(logger);
    Ok(())
}

/// Emit the filter and dispatch events of the four real-handler scenarios.
pub(super) fn emit_real_handler_filter_dispatches() -> PyResult<()> {
    emit_mixed_filter_dispatch()?;
    emit_all_rejected_filter_dispatch()?;
    emit_no_handler_dispatch()?;
    emit_propagated_filter_dispatch()
}

/// Dispatch to one accepting and one rejecting handler on a fresh logger.
fn emit_mixed_filter_dispatch() -> PyResult<()> {
    let mixed_logger = FemtoLogger::new("trace-filter-mixed".to_owned())?;
    let accepted = CollectingHandler::new();
    let rejected = CollectingHandler::new();
    mixed_logger
        .add_handler_with_filters(
            Arc::new(accepted.clone()),
            vec![Arc::new(FixedDecisionFilter { accepted: true })],
        )
        .expect("accepted handler should attach");
    mixed_logger
        .add_handler_with_filters(
            Arc::new(rejected.clone()),
            vec![Arc::new(FixedDecisionFilter { accepted: false })],
        )
        .expect("rejected handler should attach");
    mixed_logger.dispatch_to_handlers(FemtoLogRecord::new(
        "trace-filter-mixed",
        FemtoLevel::Info,
        "mixed-dispatch-payload-is-not-traced",
    ));
    assert!(mixed_logger.flush_handlers());
    assert_eq!(accepted.collected().len(), 1);
    assert!(rejected.collected().is_empty());
    Ok(())
}

/// Dispatch to a handler whose whole filter chain rejects the record.
fn emit_all_rejected_filter_dispatch() -> PyResult<()> {
    let all_rejected_logger = FemtoLogger::new("trace-filter-all-rejected".to_owned())?;
    let all_rejected = CollectingHandler::new();
    all_rejected_logger
        .add_handler_with_filters(
            Arc::new(all_rejected.clone()),
            vec![Arc::new(FixedDecisionFilter { accepted: false })],
        )
        .expect("all-rejected handler should attach");
    all_rejected_logger.dispatch_to_handlers(FemtoLogRecord::new(
        "trace-filter-all-rejected",
        FemtoLevel::Info,
        "rejected-dispatch-payload-is-not-traced",
    ));
    assert!(all_rejected_logger.flush_handlers());
    assert!(all_rejected.collected().is_empty());
    Ok(())
}

/// Dispatch from a logger that has no handlers attached.
fn emit_no_handler_dispatch() -> PyResult<()> {
    let no_handler_logger = FemtoLogger::new("trace-filter-no-handlers".to_owned())?;
    no_handler_logger.dispatch_to_handlers(FemtoLogRecord::new(
        "trace-filter-no-handlers",
        FemtoLevel::Info,
        "no-handler-payload-is-not-traced",
    ));
    assert!(no_handler_logger.flush_handlers());
    Ok(())
}

/// Dispatch from a child logger into a handler its root logger owns.
fn emit_propagated_filter_dispatch() -> PyResult<()> {
    Python::attach(|py| {
        manager::reset_manager();
        let root = manager::get_logger(py, "root")?;
        let propagated = CollectingHandler::new();
        root.borrow(py)
            .add_handler_with_filters(
                Arc::new(propagated.clone()),
                vec![Arc::new(FixedDecisionFilter { accepted: true })],
            )
            .expect("root handler should attach");
        let child =
            FemtoLogger::with_parent("trace-filter-child".to_owned(), Some("root".to_owned()))?;
        child.dispatch_to_handlers(FemtoLogRecord::new(
            "trace-filter-child",
            FemtoLevel::Info,
            "propagated-payload-is-not-traced",
        ));
        assert!(root.borrow(py).flush_handlers());
        assert_eq!(propagated.collected().len(), 1);
        manager::reset_manager();
        Ok(())
    })
}

pub(super) fn assert_event(event: &CapturedEvent, expected: &ExpectedEvent) {
    assert_eq!(
        event.fields.get("operation").map(String::as_str),
        Some(expected.operation)
    );
    assert_eq!(
        event.fields.get("handler_kind").map(String::as_str),
        Some(expected.handler_kind)
    );
    assert_eq!(
        event.fields.get(expected.outcome_field).map(String::as_str),
        Some(expected.outcome)
    );
    assert!(["python", "native", "python_and_native", "none"].contains(&expected.handler_kind));

    let actual_fields = event
        .fields
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected_fields = BTreeSet::from([
        "elapsed_us",
        "handler_kind",
        "operation",
        expected.outcome_field,
    ]);
    assert_eq!(
        actual_fields, expected_fields,
        "event fields: {:?}",
        event.fields
    );
    assert!(
        event
            .fields
            .get("elapsed_us")
            .is_some_and(|elapsed| elapsed.parse::<u128>().is_ok()),
        "elapsed_us should be numeric: {:?}",
        event.fields
    );
}

pub(super) fn assert_events(events: &[CapturedEvent], expected: &[ExpectedEvent]) {
    for expected_event in expected {
        let matching = events
            .iter()
            .filter(|event| {
                event.fields.get("operation").map(String::as_str) == Some(expected_event.operation)
                    && event.fields.get("handler_kind").map(String::as_str)
                        == Some(expected_event.handler_kind)
                    && event
                        .fields
                        .get(expected_event.outcome_field)
                        .map(String::as_str)
                        == Some(expected_event.outcome)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            matching.len(),
            expected_event.count,
            "expected {} {}/{} {}={} events; got {matching:?} in {events:?}",
            expected_event.count,
            expected_event.operation,
            expected_event.handler_kind,
            expected_event.outcome_field,
            expected_event.outcome
        );
        for event in matching {
            assert_event(event, expected_event);
        }
    }
}

pub(super) fn assert_filter_events(events: &[CapturedEvent]) {
    let filter_events = events
        .iter()
        .filter(|event| {
            event.fields.get("operation").map(String::as_str)
                == Some("producer_handler_filter_chain")
        })
        .collect::<Vec<_>>();
    let expected_filter_event_count = EXPECTED_FILTER_OUTCOMES
        .iter()
        .map(|(_, count)| *count)
        .sum::<usize>();
    assert_eq!(
        filter_events.len(),
        expected_filter_event_count,
        "filter events: {filter_events:?}"
    );

    for (outcome, expected_count) in EXPECTED_FILTER_OUTCOMES {
        let matching = filter_events
            .iter()
            .filter(|event| event.fields.get("filter_outcome").map(String::as_str) == Some(outcome))
            .collect::<Vec<_>>();
        assert_eq!(
            matching.len(),
            expected_count,
            "expected {expected_count} filter outcome {outcome}; got {matching:?}"
        );
        for event in matching {
            assert_eq!(
                event
                    .fields
                    .keys()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>(),
                BTreeSet::from(["elapsed_us", "filter_outcome", "handler_kind", "operation",]),
                "filter trace fields must stay bounded: {:?}",
                event.fields
            );
            assert_eq!(
                event.fields.get("handler_kind").map(String::as_str),
                Some("native")
            );
            assert!(
                event
                    .fields
                    .get("elapsed_us")
                    .is_some_and(|elapsed| elapsed.parse::<u128>().is_ok()),
                "elapsed_us should be numeric: {:?}",
                event.fields
            );
        }
    }
}
