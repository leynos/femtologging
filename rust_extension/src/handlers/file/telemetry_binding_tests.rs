//! Binding-level tests that the Python entry points record real waits.
//!
//! [`super::telemetry_event_tests`] pins the event shape from a synthetic
//! outcome. These tests call the bindings Python actually invokes, so the
//! handler's real wait result and the binding's counter and event recording
//! are exercised together. A regression between the worker and the binding
//! fails here rather than only in the synthetic case. Counters are read
//! through the public `wait_diagnostics()` map, so the tests pin the surface
//! an operator reads rather than the private snapshot behind it.

use super::scripted_writer::{ScriptedWriter, Stage, expect_stage, handler_with};
use super::telemetry_event_tests::EventCollector;
use super::*;
use pyo3::{Py, Python};
use std::collections::BTreeMap;
use std::sync::mpsc;
use tracing_subscriber::prelude::*;

/// Read one counter from the handler's public diagnostics map.
pub(super) fn public_count(py: Python<'_>, handler: &Py<FemtoFileHandler>, key: &str) -> u64 {
    let snapshot: BTreeMap<String, u64> = handler
        .call_method0(py, "wait_diagnostics")
        .expect("wait_diagnostics must not raise")
        .extract(py)
        .expect("wait_diagnostics must return a counter map");
    match snapshot.get(key).copied() {
        Some(count) => count,
        None => panic!("wait_diagnostics must expose {key}"),
    }
}

/// The Python `flush()` binding records a real acknowledgement timeout.
#[test]
fn the_python_flush_binding_records_a_real_ack_timeout() {
    let (entered_tx, _entered_rx) = mpsc::channel();
    let writer = ScriptedWriter::blocking(entered_tx);
    let handler = handler_with(writer.clone(), OverflowPolicy::Block);

    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let (acknowledged, recorded, handler) = tracing::subscriber::with_default(subscriber, || {
        Python::attach(|py| {
            let handler = Py::new(py, handler).expect("the handler must be a valid pyclass");
            let acknowledged: bool = handler
                .call_method0(py, "flush")
                .expect("flush must not raise")
                .extract(py)
                .expect("flush must return a bool");
            let recorded = public_count(py, &handler, "flush_ack_timed_out");
            (acknowledged, recorded, handler)
        })
    });

    assert!(
        !acknowledged,
        "a worker that never acknowledges must make flush() return False",
    );
    assert_eq!(
        recorded, 1,
        "the public map must record the real ack timeout exactly once",
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

    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let (recorded, handler) = tracing::subscriber::with_default(subscriber, || {
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
            let recorded = public_count(py, &handler, "handle_timed_out");
            (recorded, handler)
        })
    });

    assert_eq!(
        recorded, 1,
        "the public map must record the real enqueue timeout exactly once",
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

    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let (recorded, handler) = tracing::subscriber::with_default(subscriber, || {
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
            let recorded = public_count(py, &handler, "flush_enqueue_timed_out");
            (recorded, handler)
        })
    });

    assert_eq!(
        recorded, 1,
        "the public map must record the real enqueue timeout exactly once",
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

    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let (recorded, handler) = tracing::subscriber::with_default(subscriber, || {
        Python::attach(|py| {
            let handler = Py::new(py, handler).expect("the handler must be a valid pyclass");
            let acknowledged: bool = handler
                .call_method0(py, "flush")
                .expect("flush must not raise")
                .extract(py)
                .expect("flush must return a bool");
            assert!(!acknowledged, "a failed flush must report False");
            let recorded = public_count(py, &handler, "flush_failed");
            (recorded, handler)
        })
    });

    assert_eq!(
        recorded, 1,
        "the public map must record the real I/O failure exactly once",
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

/// The Python `flush()` binding records a worker that died before answering.
///
/// A worker panic after it accepted the flush request drops the acknowledgement
/// sender. That is a disconnection rather than a timeout: the request reached
/// the worker and no answer will ever arrive, so an operator must not chase a
/// slow device. The counter and the event must name it.
#[test]
fn the_python_flush_binding_records_a_real_disconnection() {
    let (entered_tx, _entered_rx) = mpsc::channel();
    let writer = ScriptedWriter::panicking_flush(entered_tx);
    let handler = handler_with(writer, OverflowPolicy::Block);

    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let (acknowledged, recorded, handler) = tracing::subscriber::with_default(subscriber, || {
        Python::attach(|py| {
            let handler = Py::new(py, handler).expect("the handler must be a valid pyclass");
            let acknowledged: bool = handler
                .call_method0(py, "flush")
                .expect("flush must not raise")
                .extract(py)
                .expect("flush must return a bool");
            let recorded = public_count(py, &handler, "flush_disconnected");
            (acknowledged, recorded, handler)
        })
    });

    assert!(
        !acknowledged,
        "a flush whose worker died must make flush() return False",
    );
    assert_eq!(
        recorded, 1,
        "the public map must record the real disconnection exactly once",
    );
    let events = collector.captured();
    assert_eq!(events.len(), 1, "one wait must emit one event: {events:?}");
    assert_eq!(events[0].fields["operation"], "handler_flush");
    assert_eq!(events[0].fields["wait_outcome"], "flush_disconnected");

    // The worker is already gone. Closing must still settle the shared
    // completion signal, so a dead worker never leaves the handler closable
    // only through a timeout.
    Python::attach(|py| {
        handler
            .call_method0(py, "close")
            .expect("close must not raise");
    });
}
