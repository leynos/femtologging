//! Binding-level tests that the Python entry points record real waits.
//!
//! [`super::telemetry_event_tests`] pins the event shape from a synthetic
//! outcome. These tests call the bindings Python actually invokes, so the
//! handler's real wait result and the binding's counter and event recording
//! are exercised together. A regression between the worker and the binding
//! fails here rather than only in the synthetic case. Counters are read
//! through the public `wait_diagnostics()` map, so the tests pin the surface
//! an operator reads rather than the private snapshot behind it.
//!
//! Every case runs through [`observe`]: it builds the handler, runs the
//! binding under a collector, and reports the counter and events the call
//! left behind. The cases differ only in the writer, the overflow policy and
//! the outcome they expect, so the "release the gauge before recording"
//! sequence is exercised in one place.

use super::scripted_writer::{ScriptedWriter, Stage, expect_stage, handler_with};
use super::telemetry_event_tests::{CapturedEvent, EventCollector};
use super::*;
use pyo3::types::PyAnyMethods;
use pyo3::{Bound, Py, PyAny, Python};
use rstest::rstest;
use std::collections::BTreeMap;
use std::sync::mpsc;
use tracing_subscriber::prelude::*;

/// What one Python binding call left behind.
struct Observation {
    /// The handler the call ran against, still open until [`shut_down`].
    handler: Py<FemtoFileHandler>,
    /// The worker the handler talks to, kept so the test can release it.
    writer: ScriptedWriter,
    /// The counter read from the public `wait_diagnostics()` map.
    recorded: u64,
    /// Every event the collector captured while the call ran.
    events: Vec<CapturedEvent>,
}

/// Read one counter from the handler's public diagnostics map.
///
/// The handler arrives as `PyAny` so the delegating rotating wrappers can use
/// the same reader: every wrapper exposes the identical `wait_diagnostics()`
/// map, and the counter name is the only thing that differs between cases.
pub(super) fn public_count(handler: &Bound<'_, PyAny>, key: &str) -> u64 {
    let snapshot: BTreeMap<String, u64> = handler
        .call_method0("wait_diagnostics")
        .expect("wait_diagnostics must not raise")
        .extract()
        .expect("wait_diagnostics must return a counter map");
    match snapshot.get(key).copied() {
        Some(count) => count,
        None => panic!("wait_diagnostics must expose {key}"),
    }
}

/// Build a handler over `make_writer`'s writer and run `act` under a collector.
///
/// The name read from the public map is the outcome key the case expects, so
/// the counter assertion and the event assertion cannot drift apart.
fn observe(
    make_writer: fn(mpsc::Sender<Stage>) -> ScriptedWriter,
    policy: OverflowPolicy,
    counter: &str,
    act: impl FnOnce(Python<'_>, &Py<FemtoFileHandler>, &mpsc::Receiver<Stage>),
) -> Observation {
    let (entered_tx, entered_rx) = mpsc::channel();
    let writer = make_writer(entered_tx);
    let handler = handler_with(writer.clone(), policy);

    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let (recorded, handler) = tracing::subscriber::with_default(subscriber, || {
        Python::attach(|py| {
            let handler = Py::new(py, handler).expect("the handler must be a valid pyclass");
            act(py, &handler, &entered_rx);
            let recorded = public_count(handler.bind(py), counter);
            (recorded, handler)
        })
    });

    Observation {
        handler,
        writer,
        recorded,
        events: collector.captured(),
    }
}

/// Call `flush()` and assert the binding reported failure.
pub(super) fn flush_fails(handler: &Bound<'_, PyAny>) {
    let acknowledged: bool = handler
        .call_method0("flush")
        .expect("flush must not raise")
        .extract()
        .expect("flush must return a bool");
    assert!(
        !acknowledged,
        "a flush that cannot be confirmed must return False",
    );
}

/// Fill the queue while the worker is parked inside its write.
///
/// The first record is accepted and parks the worker; the second one either
/// waits for capacity or is dropped, depending on the policy under test. Both
/// leave the queue full when the case's own call runs.
pub(super) fn fill_queue(handler: &Bound<'_, PyAny>, entered: &mpsc::Receiver<Stage>) {
    handler
        .call_method1("handle", ("core", "INFO", "first"))
        .expect("the first record must be accepted");
    expect_stage(entered, Stage::Write);
    handler
        .call_method1("handle", ("core", "INFO", "second"))
        .expect("the second record must fill the queue");
}

/// What a case expects one binding call to have recorded.
struct Expected {
    /// The Python class method the wait belongs to.
    operation: &'static str,
    /// The outcome key the counter and the event must carry.
    outcome: &'static str,
    /// The overflow-policy bucket the event must carry.
    bucket: &'static str,
    /// How many events the whole case must have emitted.
    total_events: usize,
}

/// One flush case: how to build the worker, the policy in force, whether the
/// queue is filled first, and what the call must record.
struct FlushCase {
    /// Builds the worker the handler talks to.
    make_writer: fn(mpsc::Sender<Stage>) -> ScriptedWriter,
    /// Overflow policy applied to the handler's queue.
    policy: OverflowPolicy,
    /// Whether the case must fill the queue before calling `flush()`.
    should_fill_queue: bool,
    /// What the case expects the call to record.
    expected: Expected,
}

/// Assert the counter and the events one wait must have produced.
fn assert_recorded(observation: &Observation, expected: &Expected) {
    assert_eq!(
        observation.recorded, 1,
        "the public map must record the real {} exactly once",
        expected.outcome,
    );
    assert_eq!(
        observation.events.len(),
        expected.total_events,
        "the wait must emit only its own events: {:?}",
        observation.events,
    );
    let matching: Vec<_> = observation
        .events
        .iter()
        .filter(|event| event.fields["wait_outcome"] == expected.outcome)
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "one wait must emit one {} event: {:?}",
        expected.outcome,
        observation.events,
    );
    assert_eq!(matching[0].fields["operation"], expected.operation);
    assert_eq!(matching[0].fields["policy_bucket"], expected.bucket);
}

/// Release the parked worker and close the handler, leaving no thread behind.
fn shut_down(observation: &Observation) {
    observation.writer.release();
    Python::attach(|py| {
        observation
            .handler
            .call_method0(py, "close")
            .expect("close must not raise");
    });
}

/// The Python `handle()` binding records a real enqueue timeout.
///
/// A full queue under a `timeout` policy is the one enqueue outcome a caller
/// can reach without the worker dying, so it is the case an operator is most
/// likely to see. The counter and the event must both name it, and the two
/// accepted records must still have emitted their own events.
#[test]
fn the_python_handle_binding_records_a_real_timeout() {
    let observation = observe(
        ScriptedWriter::blocking,
        OverflowPolicy::Timeout(Duration::from_millis(20)),
        "handle_timed_out",
        |py, handler, entered| {
            fill_queue(handler.bind(py), entered);
            assert!(
                handler
                    .call_method1(py, "handle", ("core", "INFO", "third"))
                    .is_err(),
                "a full queue under a timeout policy must raise",
            );
        },
    );

    assert_recorded(
        &observation,
        &Expected {
            operation: "handler_handle",
            outcome: "handle_timed_out",
            bucket: "timeout",
            total_events: 3,
        },
    );
    shut_down(&observation);
}

/// The Python `handle()` binding records a worker that died between calls.
///
/// The public error cannot show this: `HandlerError` reports a lost worker as
/// the same `Closed` variant a deliberate shutdown produces, so a caller only
/// learns that the call failed. The diagnostics keep the two apart, and a
/// channel that vanished while the handler still looked open is the fault an
/// operator has to act on.
#[test]
fn the_python_handle_binding_records_a_real_disconnection() {
    let observation = observe(
        ScriptedWriter::panicking_writes,
        OverflowPolicy::Block,
        "handle_disconnected",
        |py, handler, entered| {
            handler
                .call_method1(py, "handle", ("core", "INFO", "first"))
                .expect("the first record must be accepted");
            expect_stage(entered, Stage::Write);
            // The worker panics inside `write` and drops its receiver before
            // the completion signal fires, so once completion is observed the
            // next send is guaranteed to meet a disconnected channel.
            assert!(
                handler.borrow(py).completion.wait(Duration::from_secs(2)),
                "the panicking worker must still signal completion",
            );
            let error = handler
                .call_method1(py, "handle", ("core", "INFO", "second"))
                .expect_err("a lost worker must raise");
            assert!(
                error.is_instance_of::<pyo3::exceptions::PyRuntimeError>(py),
                "a lost worker must raise RuntimeError: {error}",
            );
        },
    );

    assert_recorded(
        &observation,
        &Expected {
            operation: "handler_handle",
            outcome: "handle_disconnected",
            bucket: "block",
            total_events: 2,
        },
    );
    shut_down(&observation);
}

/// Every way a Python `flush()` can end without an acknowledgement records its
/// own outcome exactly once.
///
/// The four endings point at different faults: a worker that never answered, a
/// request that never reached the worker, a worker whose flush failed, and a
/// worker that died between accepting the request and replying. The enqueue
/// case is the only one that queues records first, so it is also the only one
/// whose case emits more than the flush event.
#[rstest]
#[case::ack_timeout(FlushCase {
    make_writer: ScriptedWriter::blocking,
    policy: OverflowPolicy::Block,
    should_fill_queue: false,
    expected: Expected {
        operation: "handler_flush",
        outcome: "flush_ack_timed_out",
        bucket: "block",
        total_events: 1,
    },
})]
#[case::enqueue_timeout(FlushCase {
    make_writer: ScriptedWriter::blocking,
    policy: OverflowPolicy::Drop,
    should_fill_queue: true,
    expected: Expected {
        operation: "handler_flush",
        outcome: "flush_enqueue_timed_out",
        bucket: "drop",
        total_events: 3,
    },
})]
#[case::io_failure(FlushCase {
    make_writer: ScriptedWriter::failing_flush,
    policy: OverflowPolicy::Block,
    should_fill_queue: false,
    expected: Expected {
        operation: "handler_flush",
        outcome: "flush_failed",
        bucket: "block",
        total_events: 1,
    },
})]
#[case::disconnection(FlushCase {
    make_writer: ScriptedWriter::panicking_flush,
    policy: OverflowPolicy::Block,
    should_fill_queue: false,
    expected: Expected {
        operation: "handler_flush",
        outcome: "flush_disconnected",
        bucket: "block",
        total_events: 1,
    },
})]
fn the_python_flush_binding_records_the_real_outcome(#[case] case: FlushCase) {
    let observation = observe(
        case.make_writer,
        case.policy,
        case.expected.outcome,
        |py, handler, entered| {
            if case.should_fill_queue {
                fill_queue(handler.bind(py), entered);
            }
            flush_fails(handler.bind(py));
        },
    );

    assert_recorded(&observation, &case.expected);
    shut_down(&observation);
}
