//! Event-contract tests for the bounded handler-wait telemetry.
//!
//! The counters are the only observable output when `tracing-compat` is off;
//! with it on, each recorded wait also emits one internal event. These tests
//! pin that event's target and field set so instrumentation cannot start
//! carrying unbounded or sensitive data.

use super::scripted_writer::{ScriptedWriter, handler_with};
use super::*;
use pyo3::{Py, Python};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, mpsc};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;

/// One captured internal event.
#[derive(Clone, Debug)]
struct CapturedEvent {
    /// Event target, which decides whether the bridge ignores it.
    target: String,
    /// Rendered field values keyed by field name.
    fields: BTreeMap<String, String>,
}

/// Collects every event emitted on the current thread.
#[derive(Clone, Default)]
struct EventCollector {
    /// Captured events retained for assertions.
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

impl EventCollector {
    /// Return the captured events, failing loudly on a poisoned mutex.
    fn captured(&self) -> Vec<CapturedEvent> {
        self.events
            .lock()
            .expect("event collector must not be poisoned")
            .clone()
    }
}

/// Renders event fields into strings for assertion.
#[derive(Default)]
struct FieldVisitor {
    /// Rendered field values keyed by field name.
    fields: BTreeMap<String, String>,
}

impl Visit for FieldVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields
            .insert(field.name().to_string(), value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields
            .insert(field.name().to_string(), value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.fields
            .insert(field.name().to_string(), format!("{value:?}"));
    }
}

impl<S> Layer<S> for EventCollector
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        self.events
            .lock()
            .expect("event collector must not be poisoned")
            .push(CapturedEvent {
                target: event.metadata().target().to_string(),
                fields: visitor.fields,
            });
    }
}

/// Record one wait under a scoped subscriber and return the captured events.
fn capture_one_wait(outcome: WaitOutcome) -> Vec<CapturedEvent> {
    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    tracing::subscriber::with_default(subscriber, || {
        let diagnostics = WaitDiagnostics::new();
        WaitTimer::start().finish(
            &diagnostics,
            WaitContext {
                operation: "handler_flush",
                handler_kind: "file",
                policy_bucket: "timeout",
            },
            outcome,
        );
    });
    collector.captured()
}

/// One completed wait emits exactly one event on the ignored internal target.
#[test]
fn a_recorded_wait_emits_one_internal_event() {
    let events = capture_one_wait(WaitOutcome::FlushAckTimedOut);

    assert_eq!(events.len(), 1, "one wait must emit one event: {events:?}");
    assert_eq!(
        events[0].target, "femtologging::internal",
        "the event must use the target the tracing bridge ignores",
    );
}

/// The event carries only the bounded fields the contract names.
#[test]
fn the_event_fields_are_bounded() {
    let events = capture_one_wait(WaitOutcome::FlushAckTimedOut);
    let event = &events[0];

    assert_eq!(event.fields["operation"], "handler_flush");
    assert_eq!(event.fields["handler_kind"], "file");
    assert_eq!(event.fields["policy_bucket"], "timeout");
    assert_eq!(event.fields["wait_outcome"], "flush_ack_timed_out");
    assert!(
        event.fields["elapsed_us"].parse::<u64>().is_ok(),
        "elapsed_us must be numeric, got {:?}",
        event.fields["elapsed_us"],
    );
    assert_eq!(
        event
            .fields
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "elapsed_us",
            "handler_kind",
            "operation",
            "policy_bucket",
            "wait_outcome",
        ]),
        "the event must not carry logger names, paths, messages, or raw timeouts",
    );
}

/// The Python `flush()` binding records a real acknowledgement timeout.
///
/// `capture_one_wait` pins the event shape from a synthetic outcome. This test
/// calls the binding Python actually invokes, so the handler's real
/// `flush_outcome()` and the binding's counter and event recording are
/// exercised together. A regression between the worker and the binding fails
/// here rather than only in the synthetic case.
#[test]
fn the_python_flush_binding_records_a_real_ack_timeout() {
    let (entered_tx, _entered_rx) = mpsc::channel();
    let writer = ScriptedWriter::blocking(entered_tx);
    let handler = handler_with(writer.clone(), OverflowPolicy::Block);
    let diagnostics = Arc::clone(handler.diagnostics());

    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let (acknowledged, handler) = tracing::subscriber::with_default(subscriber, || {
        Python::attach(|py| {
            let handler = Py::new(py, handler).expect("the handler must be a valid pyclass");
            let acknowledged: bool = handler
                .call_method0(py, "flush")
                .expect("flush must not raise")
                .extract(py)
                .expect("flush must return a bool");
            (acknowledged, handler)
        })
    });

    assert!(
        !acknowledged,
        "a worker that never acknowledges must make flush() return False",
    );
    assert_eq!(
        diagnostics.snapshot().count(WaitOutcome::FlushAckTimedOut),
        1,
        "the binding must record the real ack timeout exactly once",
    );
    let events = collector.captured();
    assert_eq!(events.len(), 1, "one wait must emit one event: {events:?}");
    assert_eq!(events[0].fields["operation"], "handler_flush");
    assert_eq!(events[0].fields["wait_outcome"], "flush_ack_timed_out");

    writer.release();
    Python::attach(|py| {
        handler
            .call_method0(py, "close")
            .expect("close must not raise");
    });
}

/// The counters still record when no subscriber is installed.
#[test]
fn counters_record_without_a_subscriber() {
    let diagnostics = WaitDiagnostics::new();
    WaitTimer::start().finish(
        &diagnostics,
        WaitContext {
            operation: "handler_handle",
            handler_kind: "file",
            policy_bucket: "drop",
        },
        WaitOutcome::HandleAccepted,
    );

    assert_eq!(
        diagnostics.snapshot().count(WaitOutcome::HandleAccepted),
        1,
        "the counters must not depend on a tracing subscriber",
    );
}
