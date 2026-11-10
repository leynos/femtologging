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
use pyo3::{Py, Python};
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
            let recorded = public_count(py, &handler, counter);
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
fn flush_fails(py: Python<'_>, handler: &Py<FemtoFileHandler>) {
    let acknowledged: bool = handler
        .call_method0(py, "flush")
        .expect("flush must not raise")
        .extract(py)
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
fn fill_queue(py: Python<'_>, handler: &Py<FemtoFileHandler>, entered: &mpsc::Receiver<Stage>) {
    handler
        .call_method1(py, "handle", ("core", "INFO", "first"))
        .expect("the first record must be accepted");
    expect_stage(entered, Stage::Write);
    handler
        .call_method1(py, "handle", ("core", "INFO", "second"))
        .expect("the second record must fill the queue");
}

/// Assert the counter and the single event one wait must have produced.
fn assert_recorded(
    observation: &Observation,
    operation: &str,
    outcome: &str,
    bucket: &str,
    total_events: usize,
) {
    assert_eq!(
        observation.recorded, 1,
        "the public map must record the real {outcome} exactly once",
    );
    assert_eq!(
        observation.events.len(),
        total_events,
        "the wait must emit only its own events: {:?}",
        observation.events,
    );
    let matching: Vec<_> = observation
        .events
        .iter()
        .filter(|event| event.fields["wait_outcome"] == outcome)
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "one wait must emit one {outcome} event: {:?}",
        observation.events,
    );
    assert_eq!(matching[0].fields["operation"], operation);
    assert_eq!(matching[0].fields["policy_bucket"], bucket);
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
            fill_queue(py, handler, entered);
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
        "handler_handle",
        "handle_timed_out",
        "timeout",
        3,
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
#[case::ack_timeout(
    ScriptedWriter::blocking,
    OverflowPolicy::Block,
    "flush_ack_timed_out",
    "block",
    false,
    1
)]
#[case::enqueue_timeout(
    ScriptedWriter::blocking,
    OverflowPolicy::Drop,
    "flush_enqueue_timed_out",
    "drop",
    true,
    3
)]
#[case::io_failure(
    ScriptedWriter::failing_flush,
    OverflowPolicy::Block,
    "flush_failed",
    "block",
    false,
    1
)]
#[case::disconnection(
    ScriptedWriter::panicking_flush,
    OverflowPolicy::Block,
    "flush_disconnected",
    "block",
    false,
    1
)]
fn the_python_flush_binding_records_the_real_outcome(
    #[case] make_writer: fn(mpsc::Sender<Stage>) -> ScriptedWriter,
    #[case] policy: OverflowPolicy,
    #[case] expected_outcome: &'static str,
    #[case] expected_bucket: &'static str,
    #[case] should_fill_queue: bool,
    #[case] expected_events: usize,
) {
    let observation = observe(
        make_writer,
        policy,
        expected_outcome,
        |py, handler, entered| {
            if should_fill_queue {
                fill_queue(py, handler, entered);
            }
            flush_fails(py, handler);
        },
    );

    assert_recorded(
        &observation,
        "handler_flush",
        expected_outcome,
        expected_bucket,
        expected_events,
    );
    shut_down(&observation);
}
