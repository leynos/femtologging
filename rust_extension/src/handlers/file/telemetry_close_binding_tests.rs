//! Binding-level tests for the outcomes the Python `close()` wait records.
//!
//! The close wait has two outcomes a successful run does not show: a deadline
//! that expires while the worker is still parked, and a worker that panicked
//! on its way out. Both are recorded through the public diagnostics map and
//! the internal trace event, so these tests call the Python binding rather
//! than the private snapshot.

use super::scripted_writer::{ScriptedWriter, Stage, expect_stage, handler_with};
use super::telemetry_binding_tests::public_count;
use super::telemetry_event_tests::EventCollector;
use super::*;
use pyo3::{Py, Python};
use std::sync::mpsc;
use tracing_subscriber::prelude::*;

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

    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let (recorded_timeouts, handler) = tracing::subscriber::with_default(subscriber, || {
        Python::attach(|py| {
            let handler = Py::new(py, handler).expect("the handler must be a valid pyclass");
            handler
                .call_method0(py, "close")
                .expect("a timed-out close must not raise");
            let recorded = public_count(handler.bind(py), "close_timed_out");
            (recorded, handler)
        })
    });
    assert_eq!(
        recorded_timeouts, 1,
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
    let recorded_completions = tracing::subscriber::with_default(later_subscriber, || {
        Python::attach(|py| {
            handler
                .call_method0(py, "close")
                .expect("the later close must not raise");
            public_count(handler.bind(py), "close_completed")
        })
    });

    assert_eq!(recorded_timeouts, 1, "the timeout must stay counted once",);
    assert_eq!(
        recorded_completions, 1,
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

/// Poll the public in-flight gauge until it reaches `expected`.
///
/// The last reading is returned so the caller asserts on what it actually saw
/// rather than on the fact that the deadline passed.
fn await_gauge(handler: &Py<FemtoFileHandler>, expected: u64) -> u64 {
    let deadline = std::time::Instant::now() + Duration::from_millis(800);
    loop {
        let gauge = Python::attach(|py| public_count(handler.bind(py), "waits_in_flight"));
        if gauge == expected || std::time::Instant::now() >= deadline {
            return gauge;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Two closers share one shutdown, and both are counted while they wait.
///
/// A closer that arrives after another has taken the worker state owns no join
/// handle, so the shared completion signal is its only evidence that the worker
/// stopped. Parking the worker inside `write` holds both closers in that wait
/// at once: the gauge must show two waits in flight before the release, and
/// both closers must then record the completion that release produces.
#[test]
fn concurrent_closers_wait_on_one_shutdown() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let writer = ScriptedWriter::blocking(entered_tx);
    let handler = handler_with(writer.clone(), OverflowPolicy::Block);

    let handler = Python::attach(|py| {
        let handler = Py::new(py, handler).expect("the handler must be a valid pyclass");
        handler
            .call_method1(py, "handle", ("core", "INFO", "park"))
            .expect("the parking record must be accepted");
        handler
    });
    expect_stage(&entered_rx, Stage::Write);

    let closers: Vec<_> = (0..2)
        .map(|_| {
            let handler = Python::attach(|py| handler.clone_ref(py));
            std::thread::spawn(move || {
                Python::attach(|py| {
                    handler
                        .call_method0(py, "close")
                        .expect("a concurrent close must not raise");
                });
            })
        })
        .collect();

    // Both closers release the interpreter lock before waiting, so polling
    // from this thread cannot hold either of them up.
    assert_eq!(
        await_gauge(&handler, 2),
        2,
        "both closers must be counted while they wait on the shared shutdown",
    );

    writer.release();
    for closer in closers {
        closer.join().expect("a closer thread must not panic");
    }

    let (completed, timed_out, gauge) = Python::attach(|py| {
        let bound = handler.bind(py);
        (
            public_count(bound, "close_completed"),
            public_count(bound, "close_timed_out"),
            public_count(bound, "waits_in_flight"),
        )
    });

    assert_eq!(
        completed, 2,
        "both closers must record the completion the release produced",
    );
    assert_eq!(
        timed_out, 0,
        "neither closer may time out once the worker is released",
    );
    assert_eq!(gauge, 0, "every finished wait must leave the gauge");
}

/// The Python `close()` binding records a real worker panic.
#[test]
fn the_python_close_binding_records_a_real_panic() {
    let (entered_tx, _entered_rx) = mpsc::channel();
    let writer = ScriptedWriter::panicking_flush(entered_tx);
    let handler = handler_with(writer, OverflowPolicy::Block);

    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let recorded = tracing::subscriber::with_default(subscriber, || {
        Python::attach(|py| {
            let handler = Py::new(py, handler).expect("the handler must be a valid pyclass");
            handler
                .call_method0(py, "close")
                .expect("a panicking worker must not raise from close()");
            public_count(handler.bind(py), "close_panicked")
        })
    });

    assert_eq!(
        recorded, 1,
        "a worker that panicked must be recorded as a panic",
    );
    let events = collector.captured();
    assert_eq!(events.len(), 1, "one wait must emit one event: {events:?}");
    assert_eq!(events[0].fields["operation"], "handler_close");
    assert_eq!(events[0].fields["wait_outcome"], "close_panicked");
}
