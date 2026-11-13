//! Tests that reading the wait diagnostics has no observable side effects.
//!
//! Diagnostics exist so an operator can watch a handler without perturbing
//! it. These tests call the accessor through the Python binding and check both
//! surfaces that would reveal a side effect: the worker command queue and the
//! internal tracing events.

use super::scripted_writer::{ScriptedWriter, Stage, expect_stage, handler_with};
use super::telemetry_event_tests::EventCollector;
use super::*;
use pyo3::{Py, Python};
use std::collections::BTreeMap;
use std::sync::{Arc, mpsc};
use tracing_subscriber::prelude::*;

/// Count the commands waiting in the handler's queue.
///
/// The scripted writer reports the stages the worker reaches, but a command
/// the accessor queued would only reach the worker later, so the stage channel
/// could miss it. Reading the queue observes the command boundary directly: a
/// command that was queued is counted whether or not the worker has picked it
/// up yet.
fn queued_commands(py: Python<'_>, handler: &Py<FemtoFileHandler>) -> usize {
    let handler = handler.borrow(py);
    handler
        .tx
        .lock()
        .as_ref()
        .map_or(0, crate::sync::Sender::len)
}

/// Reading `wait_diagnostics()` is an observation, not a wait.
///
/// A scrape loop must not inflate the numbers it reports: the accessor takes no
/// worker command and emits no internal event. The worker is parked inside
/// `write`, so whatever the reads did to the command queue would still be
/// there when they return; the collector reports any event emitted. After the
/// reads, one real `handle()` call must leave exactly one queued command, which
/// proves the queue reading above can see a command at all.
#[test]
fn the_python_diagnostics_read_is_an_observation() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let writer = ScriptedWriter::blocking(entered_tx);
    let handler = handler_with(writer.clone(), OverflowPolicy::Block);
    let diagnostics = Arc::clone(handler.diagnostics());

    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let (handler, accepted, (queued_after_reads, queued_by_control), events_while_observing) =
        tracing::subscriber::with_default(subscriber, || {
            Python::attach(|py| {
                let handler = Py::new(py, handler).expect("the handler must be a valid pyclass");
                handler
                    .call_method1(py, "handle", ("core", "INFO", "one"))
                    .expect("the seeded record must be accepted");
                // The worker takes the command before it enters `write`, so a
                // parked worker leaves the queue empty and only a later send
                // can fill it.
                expect_stage(&entered_rx, Stage::Write);

                let first: BTreeMap<String, u64> = handler
                    .call_method0(py, "wait_diagnostics")
                    .expect("wait_diagnostics must not raise")
                    .extract(py)
                    .expect("wait_diagnostics must return a counter map");
                for _ in 0..3 {
                    let repeated: BTreeMap<String, u64> = handler
                        .call_method0(py, "wait_diagnostics")
                        .expect("a repeated read must not raise")
                        .extract(py)
                        .expect("wait_diagnostics must return a counter map");
                    assert_eq!(repeated, first, "a read must not move a counter");
                }
                let queued_after_reads = queued_commands(py, &handler);
                let accepted = diagnostics.snapshot().count(WaitOutcome::HandleAccepted);
                let events = collector.captured().len();

                // Positive control for the check above: a real command does
                // reach the queue, so an empty reading is evidence and not a
                // probe that can never see anything. The worker is still
                // parked, so the control command cannot be taken before it is
                // counted.
                handler
                    .call_method1(py, "handle", ("core", "INFO", "two"))
                    .expect("the control record must be accepted");
                let queued_by_control = queued_commands(py, &handler);
                (
                    handler,
                    accepted,
                    (queued_after_reads, queued_by_control),
                    events,
                )
            })
        });

    assert_eq!(
        queued_after_reads, 0,
        "a read must not enqueue a worker command",
    );
    assert_eq!(
        queued_by_control, 1,
        "the control record must be the one queued command",
    );
    assert_eq!(
        accepted, 1,
        "the seeded handle must be the only wait recorded while observing",
    );
    assert_eq!(
        events_while_observing, 1,
        "only the seeded handle may emit an event while observing",
    );

    writer.release();
    Python::attach(|py| {
        handler
            .call_method0(py, "close")
            .expect("close must not raise");
    });
}
