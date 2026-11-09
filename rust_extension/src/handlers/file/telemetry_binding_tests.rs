//! Binding-level tests that the Python entry points record real waits.
//!
//! [`super::telemetry_event_tests`] pins the event shape from a synthetic
//! outcome. These tests call the bindings Python actually invokes, so the
//! handler's real wait result and the binding's counter and event recording
//! are exercised together. A regression between the worker and the binding
//! fails here rather than only in the synthetic case.

use super::scripted_writer::{ScriptedWriter, Stage, expect_stage, handler_with};
use super::telemetry_event_tests::EventCollector;
use super::*;
use pyo3::{Py, Python};
use std::sync::{Arc, mpsc};
use tracing_subscriber::prelude::*;

/// The Python `flush()` binding records a real acknowledgement timeout.
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

/// The Python `handle()` binding records a real enqueue timeout.
///
/// A full queue under a `timeout` policy is the one enqueue outcome a caller
/// can reach without the worker dying, so it is the case an operator is most
/// likely to see. The counter and the event must both name it.
#[test]
fn the_python_handle_binding_records_a_real_timeout() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let writer = ScriptedWriter::blocking(entered_tx);
    let handler = handler_with(
        writer.clone(),
        OverflowPolicy::Timeout(Duration::from_millis(20)),
    );
    let diagnostics = Arc::clone(handler.diagnostics());

    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let handler = tracing::subscriber::with_default(subscriber, || {
        Python::attach(|py| {
            let handler = Py::new(py, handler).expect("the handler must be a valid pyclass");
            handler
                .call_method1(py, "handle", ("core", "INFO", "first"))
                .expect("the first record must be accepted");
            expect_stage(&entered_rx, Stage::Write);
            handler
                .call_method1(py, "handle", ("core", "INFO", "second"))
                .expect("the queued record must be accepted");
            assert!(
                handler
                    .call_method1(py, "handle", ("core", "INFO", "third"))
                    .is_err(),
                "a full queue under a timeout policy must raise",
            );
            handler
        })
    });

    assert_eq!(
        diagnostics.snapshot().count(WaitOutcome::HandleTimedOut),
        1,
        "the binding must record the real enqueue timeout exactly once",
    );
    let events = collector.captured();
    let timed_out: Vec<_> = events
        .iter()
        .filter(|event| event.fields["wait_outcome"] == "handle_timed_out")
        .collect();
    assert_eq!(
        timed_out.len(),
        1,
        "one timeout must emit one event: {events:?}"
    );
    assert_eq!(timed_out[0].fields["operation"], "handler_handle");
    assert_eq!(timed_out[0].fields["policy_bucket"], "timeout");

    writer.release();
    Python::attach(|py| {
        handler
            .call_method0(py, "close")
            .expect("close must not raise");
    });
}

/// The Python `flush()` binding records a real enqueue timeout.
///
/// A flush that never reaches the worker because the queue stayed full is a
/// different fault from an acknowledgement timeout: the worker never saw the
/// request. The counter and the event must name the enqueue timeout rather
/// than folding it into the acknowledgement case.
#[test]
fn the_python_flush_binding_records_a_real_enqueue_timeout() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let writer = ScriptedWriter::blocking(entered_tx);
    let handler = handler_with(writer.clone(), OverflowPolicy::Drop);
    let diagnostics = Arc::clone(handler.diagnostics());

    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let handler = tracing::subscriber::with_default(subscriber, || {
        Python::attach(|py| {
            let handler = Py::new(py, handler).expect("the handler must be a valid pyclass");
            handler
                .call_method1(py, "handle", ("core", "INFO", "first"))
                .expect("the first record must be accepted");
            expect_stage(&entered_rx, Stage::Write);
            handler
                .call_method1(py, "handle", ("core", "INFO", "second"))
                .expect("the second record must fill the queue");
            let acknowledged: bool = handler
                .call_method0(py, "flush")
                .expect("flush must not raise")
                .extract(py)
                .expect("flush must return a bool");
            assert!(
                !acknowledged,
                "a flush that never reached the worker must return False",
            );
            handler
        })
    });

    assert_eq!(
        diagnostics
            .snapshot()
            .count(WaitOutcome::FlushEnqueueTimedOut),
        1,
        "the binding must record the real enqueue timeout exactly once",
    );
    let events = collector.captured();
    let enqueue_timeouts: Vec<_> = events
        .iter()
        .filter(|event| event.fields["wait_outcome"] == "flush_enqueue_timed_out")
        .collect();
    assert_eq!(
        enqueue_timeouts.len(),
        1,
        "one enqueue timeout must emit one event: {events:?}"
    );
    assert_eq!(enqueue_timeouts[0].fields["operation"], "handler_flush");
    assert_eq!(enqueue_timeouts[0].fields["policy_bucket"], "drop");

    writer.release();
    Python::attach(|py| {
        handler
            .call_method0(py, "close")
            .expect("close must not raise");
    });
}

/// The Python `flush()` binding records a real I/O failure.
///
/// A failed worker flush is different from a timeout: the worker answered, and
/// the answer was an error. The counter and the event must say so rather than
/// folding it into the timeout case.
#[test]
fn the_python_flush_binding_records_a_real_io_failure() {
    let (entered_tx, _entered_rx) = mpsc::channel();
    let writer = ScriptedWriter::failing_flush(entered_tx);
    let handler = handler_with(writer.clone(), OverflowPolicy::Block);
    let diagnostics = Arc::clone(handler.diagnostics());

    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let handler = tracing::subscriber::with_default(subscriber, || {
        Python::attach(|py| {
            let handler = Py::new(py, handler).expect("the handler must be a valid pyclass");
            let acknowledged: bool = handler
                .call_method0(py, "flush")
                .expect("flush must not raise")
                .extract(py)
                .expect("flush must return a bool");
            assert!(!acknowledged, "a failed flush must report False");
            handler
        })
    });

    assert_eq!(
        diagnostics.snapshot().count(WaitOutcome::FlushFailed),
        1,
        "the binding must record the real I/O failure exactly once",
    );
    let events = collector.captured();
    assert_eq!(events.len(), 1, "one wait must emit one event: {events:?}");
    assert_eq!(events[0].fields["operation"], "handler_flush");
    assert_eq!(events[0].fields["wait_outcome"], "flush_failed");

    writer.release();
    Python::attach(|py| {
        handler
            .call_method0(py, "close")
            .expect("close must not raise");
    });
}

/// The Python `close()` binding records a real timeout and a later completion.
///
/// A closer that timed out leaves the shared completion signal behind. A second
/// `close()` must still observe the worker stop and record that separately,
/// because a caller that retries needs to know whether the retry settled it.
#[test]
fn the_python_close_binding_records_a_timeout_then_a_completion() {
    let (entered_tx, _entered_rx) = mpsc::channel();
    let writer = ScriptedWriter::blocking(entered_tx);
    let handler = handler_with(writer.clone(), OverflowPolicy::Block);
    let diagnostics = Arc::clone(handler.diagnostics());

    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let handler = tracing::subscriber::with_default(subscriber, || {
        Python::attach(|py| {
            let handler = Py::new(py, handler).expect("the handler must be a valid pyclass");
            handler
                .call_method0(py, "close")
                .expect("a timed-out close must not raise");
            handler
        })
    });
    assert_eq!(
        diagnostics.snapshot().count(WaitOutcome::CloseTimedOut),
        1,
        "a worker blocked in its terminal flush must record a timeout",
    );

    writer.release();
    let completed = Python::attach(|py| handler.borrow(py).completion.wait(Duration::from_secs(2)));
    assert!(
        completed,
        "the released worker must still signal completion",
    );

    // The second close runs under its own subscriber: `with_default` scopes
    // event capture, so a collector installed only for the first close would
    // see the timeout and miss the completion that follows it.
    let later_collector = EventCollector::default();
    let later_subscriber = tracing_subscriber::registry().with(later_collector.clone());
    tracing::subscriber::with_default(later_subscriber, || {
        Python::attach(|py| {
            handler
                .call_method0(py, "close")
                .expect("the later close must not raise");
        })
    });

    let snapshot = diagnostics.snapshot();
    assert_eq!(
        snapshot.count(WaitOutcome::CloseTimedOut),
        1,
        "the timeout must stay counted once",
    );
    assert_eq!(
        snapshot.count(WaitOutcome::CloseCompleted),
        1,
        "the later close must record the completion it observed",
    );
    let first_outcomes: Vec<_> = collector
        .captured()
        .iter()
        .map(|event| event.fields["wait_outcome"].clone())
        .collect();
    assert_eq!(
        first_outcomes,
        ["close_timed_out"],
        "the first close must emit only its timeout",
    );
    let later_outcomes: Vec<_> = later_collector
        .captured()
        .iter()
        .map(|event| event.fields["wait_outcome"].clone())
        .collect();
    assert_eq!(
        later_outcomes,
        ["close_completed"],
        "the later close must emit the completion it observed",
    );
}

/// The Python `close()` binding records a real worker panic.
#[test]
fn the_python_close_binding_records_a_real_panic() {
    let (entered_tx, _entered_rx) = mpsc::channel();
    let writer = ScriptedWriter::panicking_flush(entered_tx);
    let handler = handler_with(writer, OverflowPolicy::Block);
    let diagnostics = Arc::clone(handler.diagnostics());

    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    tracing::subscriber::with_default(subscriber, || {
        Python::attach(|py| {
            let handler = Py::new(py, handler).expect("the handler must be a valid pyclass");
            handler
                .call_method0(py, "close")
                .expect("a panicking worker must not raise from close()");
        })
    });

    assert_eq!(
        diagnostics.snapshot().count(WaitOutcome::ClosePanicked),
        1,
        "a worker that panicked must be recorded as a panic",
    );
    let events = collector.captured();
    assert_eq!(events.len(), 1, "one wait must emit one event: {events:?}");
    assert_eq!(events[0].fields["operation"], "handler_close");
    assert_eq!(events[0].fields["wait_outcome"], "close_panicked");
}
