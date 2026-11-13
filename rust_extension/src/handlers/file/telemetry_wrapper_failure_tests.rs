//! Binding-level tests for the failure paths the rotating wrappers record.
//!
//! The rotating and timed-rotating wrappers delegate their waits to the core
//! file handler, so they share its outcome classification and differ only in
//! the `handler_kind` and operation labels. These cases drive each failure
//! through the wrapper's own Python method and assert the counter, the event
//! and the gauge, so a wrapper that stopped counting or labelling a failure
//! fails here rather than only in the plain file handler's tests.
//!
//! The successful paths live in [`super::telemetry_wrapper_binding_tests`],
//! which also builds the wrappers these cases use.

use std::sync::mpsc;
use std::time::Duration;

use pyo3::types::PyAnyMethods;
use pyo3::{Bound, Py, PyAny, Python};
use rstest::rstest;
use tracing_subscriber::prelude::*;

use super::OverflowPolicy;
use super::scripted_writer::{ScriptedWriter, Stage};
use super::telemetry_binding_tests::{fill_queue, flush_fails, public_count};
use super::telemetry_event_tests::{CapturedEvent, EventCollector};
use super::telemetry_wrapper_binding_tests::{rotating_over_writer, timed_rotating_over_writer};

/// The failure a wrapper case plays out, and what it must record.
#[derive(Clone, Copy, Debug)]
enum Failure {
    /// The worker's flush reports an I/O error.
    FlushFails,
    /// The worker panics inside its terminal flush.
    ClosePanics,
    /// The queue stays full past the enqueue deadline.
    HandleTimedOut,
    /// The worker accepts a flush but never answers it.
    FlushAckTimedOut,
    /// The flush request never reaches the worker.
    FlushEnqueueTimedOut,
    /// The worker does not stop within the close deadline.
    CloseTimedOut,
}

impl Failure {
    /// Build the writer that plays out this failure.
    ///
    /// Every timeout case needs a worker that stays parked; the two failure
    /// cases need a worker that answers immediately and then fails.
    fn writer(self, entered: mpsc::Sender<Stage>) -> ScriptedWriter {
        match self {
            Self::FlushFails => ScriptedWriter::failing_flush(entered),
            Self::ClosePanics => ScriptedWriter::panicking_flush(entered),
            Self::HandleTimedOut
            | Self::FlushAckTimedOut
            | Self::FlushEnqueueTimedOut
            | Self::CloseTimedOut => ScriptedWriter::blocking(entered),
        }
    }

    /// The overflow policy the queued case needs.
    ///
    /// An enqueue timeout only exists under a timed policy, and an enqueue
    /// refusal only exists under `drop`; the remaining cases are driven by the
    /// wait deadline and use `block` so the queue is never the cause.
    fn policy(self) -> OverflowPolicy {
        match self {
            Self::HandleTimedOut => OverflowPolicy::Timeout(Duration::from_millis(20)),
            Self::FlushEnqueueTimedOut => OverflowPolicy::Drop,
            _ => OverflowPolicy::Block,
        }
    }

    /// The `policy_bucket` the event must carry.
    const fn bucket(self) -> &'static str {
        match self {
            Self::HandleTimedOut => "timeout",
            Self::FlushEnqueueTimedOut => "drop",
            _ => "block",
        }
    }

    /// The operation the failing wait belongs to.
    const fn operation(self) -> &'static str {
        match self {
            Self::FlushFails | Self::FlushAckTimedOut | Self::FlushEnqueueTimedOut => {
                "handler_flush"
            }
            Self::HandleTimedOut => "handler_handle",
            Self::ClosePanics | Self::CloseTimedOut => "handler_close",
        }
    }

    /// The counter key and event outcome the failing wait must record.
    const fn outcome(self) -> &'static str {
        match self {
            Self::FlushFails => "flush_failed",
            Self::ClosePanics => "close_panicked",
            Self::HandleTimedOut => "handle_timed_out",
            Self::FlushAckTimedOut => "flush_ack_timed_out",
            Self::FlushEnqueueTimedOut => "flush_enqueue_timed_out",
            Self::CloseTimedOut => "close_timed_out",
        }
    }

    /// Whether the case must fill the queue before its own call.
    ///
    /// Both enqueue failures need the queue to stay full while the call waits;
    /// the other cases must meet an idle queue so the wait reaches the worker.
    const fn should_fill_queue(self) -> bool {
        matches!(self, Self::HandleTimedOut | Self::FlushEnqueueTimedOut)
    }

    /// Whether the failing call already closed the handler.
    const fn closes(self) -> bool {
        matches!(self, Self::ClosePanics | Self::CloseTimedOut)
    }

    /// Whether a second close is needed to record the completion the first
    /// timed-out close could not observe.
    const fn needs_retry_close(self) -> bool {
        matches!(self, Self::CloseTimedOut)
    }

    /// How many events the whole case emits before the handler is closed.
    ///
    /// The queue-filled cases accept two records first, and each accepted
    /// record is a wait of its own.
    const fn event_count(self) -> usize {
        if self.should_fill_queue() { 3 } else { 1 }
    }

    /// Drive the failing call through the wrapper's Python method.
    fn act(self, handler: &Bound<'_, PyAny>, entered: &mpsc::Receiver<Stage>) {
        if self.should_fill_queue() {
            fill_queue(handler, entered);
        }
        match self {
            Self::FlushFails | Self::FlushAckTimedOut | Self::FlushEnqueueTimedOut => {
                flush_fails(handler);
            }
            Self::HandleTimedOut => {
                assert!(
                    handler
                        .call_method1("handle", ("core", "INFO", "third"))
                        .is_err(),
                    "a full queue under a timed policy must raise",
                );
            }
            Self::ClosePanics | Self::CloseTimedOut => {
                handler
                    .call_method0("close")
                    .expect("a failing worker must not raise from close()");
            }
        }
    }
}

/// One wrapper failure case: the wrapper, its expected kind, and the failure.
struct FailureCase {
    /// Builds the wrapper under test over the case's scripted writer.
    make_wrapper: fn(Python<'_>, ScriptedWriter, OverflowPolicy) -> Py<PyAny>,
    /// The `handler_kind` every event must carry.
    expected_kind: &'static str,
    /// The failure the worker plays out.
    failure: Failure,
}

/// Assert the one event the failing wait emitted carries its labels.
fn assert_failure_event(events: &[CapturedEvent], case: &FailureCase) {
    assert_eq!(
        events.len(),
        case.failure.event_count(),
        "the case must emit only its own events: {events:?}",
    );
    let matching: Vec<_> = events
        .iter()
        .filter(|event| event.fields["wait_outcome"] == case.failure.outcome())
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "one wait must emit one {} event: {events:?}",
        case.failure.outcome(),
    );
    assert_eq!(
        matching[0].fields["handler_kind"], case.expected_kind,
        "the failure must carry the wrapper's own kind: {events:?}",
    );
    assert_eq!(matching[0].fields["operation"], case.failure.operation());
    assert_eq!(matching[0].fields["policy_bucket"], case.failure.bucket());
}

/// A wrapper's failing wait is counted and labelled with its own kind.
#[rstest]
#[case::rotating_handle_timed_out(FailureCase {
    make_wrapper: rotating_over_writer,
    expected_kind: "rotating",
    failure: Failure::HandleTimedOut,
})]
#[case::rotating_flush_ack_timed_out(FailureCase {
    make_wrapper: rotating_over_writer,
    expected_kind: "rotating",
    failure: Failure::FlushAckTimedOut,
})]
#[case::rotating_flush_enqueue_timed_out(FailureCase {
    make_wrapper: rotating_over_writer,
    expected_kind: "rotating",
    failure: Failure::FlushEnqueueTimedOut,
})]
#[case::rotating_flush_failed(FailureCase {
    make_wrapper: rotating_over_writer,
    expected_kind: "rotating",
    failure: Failure::FlushFails,
})]
#[case::rotating_close_timed_out(FailureCase {
    make_wrapper: rotating_over_writer,
    expected_kind: "rotating",
    failure: Failure::CloseTimedOut,
})]
#[case::rotating_close_panicked(FailureCase {
    make_wrapper: rotating_over_writer,
    expected_kind: "rotating",
    failure: Failure::ClosePanics,
})]
#[case::timed_rotating_handle_timed_out(FailureCase {
    make_wrapper: timed_rotating_over_writer,
    expected_kind: "timed_rotating",
    failure: Failure::HandleTimedOut,
})]
#[case::timed_rotating_flush_ack_timed_out(FailureCase {
    make_wrapper: timed_rotating_over_writer,
    expected_kind: "timed_rotating",
    failure: Failure::FlushAckTimedOut,
})]
#[case::timed_rotating_flush_enqueue_timed_out(FailureCase {
    make_wrapper: timed_rotating_over_writer,
    expected_kind: "timed_rotating",
    failure: Failure::FlushEnqueueTimedOut,
})]
#[case::timed_rotating_flush_failed(FailureCase {
    make_wrapper: timed_rotating_over_writer,
    expected_kind: "timed_rotating",
    failure: Failure::FlushFails,
})]
#[case::timed_rotating_close_timed_out(FailureCase {
    make_wrapper: timed_rotating_over_writer,
    expected_kind: "timed_rotating",
    failure: Failure::CloseTimedOut,
})]
#[case::timed_rotating_close_panicked(FailureCase {
    make_wrapper: timed_rotating_over_writer,
    expected_kind: "timed_rotating",
    failure: Failure::ClosePanics,
})]
fn each_wrapper_labels_its_failure_waits(#[case] case: FailureCase) {
    let (entered_tx, entered_rx) = mpsc::channel();
    let writer = case.failure.writer(entered_tx);

    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let (recorded, handler) = tracing::subscriber::with_default(subscriber, || {
        Python::attach(|py| {
            let handler = (case.make_wrapper)(py, writer.clone(), case.failure.policy());
            case.failure.act(handler.bind(py), &entered_rx);
            let recorded = public_count(handler.bind(py), case.failure.outcome());
            (recorded, handler)
        })
    });

    assert_eq!(
        recorded,
        1,
        "the wrapper must count its {}: {recorded}",
        case.failure.outcome(),
    );
    assert_failure_event(&collector.captured(), &case);

    // Release the parked worker and finish the shutdown outside the
    // subscriber, so the cleanup's own event cannot be counted above. A
    // timed-out close is retried so the shared completion is still observed.
    writer.release();
    let retried = Python::attach(|py| {
        if case.failure.needs_retry_close() {
            handler
                .call_method0(py, "close")
                .expect("the retried close must not raise");
            return Some(public_count(handler.bind(py), "close_completed"));
        }
        if !case.failure.closes() {
            handler
                .call_method0(py, "close")
                .expect("the later close must not raise");
        }
        None
    });
    if let Some(completed) = retried {
        assert_eq!(
            completed, 1,
            "the retried close must record the completion it observed",
        );
    }
}
