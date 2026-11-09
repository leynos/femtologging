//! Tests that reading the wait diagnostics has no observable side effects.
//!
//! Diagnostics exist so an operator can watch a handler without perturbing
//! it. These tests call the accessor through the Python binding and check both
//! surfaces that would reveal a side effect: the worker command channel and
//! the internal tracing events.

use super::scripted_writer::{ScriptedWriter, Stage, expect_stage, handler_with};
use super::telemetry_event_tests::EventCollector;
use super::*;
use pyo3::{Py, Python};
use std::collections::BTreeMap;
use std::sync::{Arc, mpsc};
use tracing_subscriber::prelude::*;

/// Reading `wait_diagnostics()` is an observation, not a wait.
///
/// A scrape loop must not inflate the numbers it reports: the accessor takes
/// no worker command and emits no internal event. The scripted writer reports
/// any command the handler sends, and the collector reports any event emitted,
/// so either side effect would show up here.
#[test]
fn the_python_diagnostics_read_is_an_observation() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let writer = ScriptedWriter::blocking_then_panicking_flush(entered_tx);
    let handler = handler_with(writer.clone(), OverflowPolicy::Block);
    let diagnostics = Arc::clone(handler.diagnostics());

    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let handler = tracing::subscriber::with_default(subscriber, || {
        Python::attach(|py| {
            let handler = Py::new(py, handler).expect("the handler must be a valid pyclass");
            handler
                .call_method1(py, "handle", ("core", "INFO", "one"))
                .expect("the seeded record must be accepted");
            expect_stage(&entered_rx, Stage::Write);
            // One formatted record can span more than one `write` call, so let
            // the worker go quiet before a later stage is treated as a
            // command the read sent.
            while entered_rx
                .recv_timeout(std::time::Duration::from_millis(200))
                .is_ok()
            {}

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
            assert!(
                entered_rx.try_recv().is_err(),
                "a read must not send a worker command",
            );
            handler
        })
    });

    assert_eq!(
        diagnostics.snapshot().count(WaitOutcome::HandleAccepted),
        1,
        "the seeded handle must be the only recorded wait",
    );
    let events = collector.captured();
    assert_eq!(
        events.len(),
        1,
        "only the seeded handle may emit an event: {events:?}",
    );
    assert_eq!(events[0].fields["operation"], "handler_handle");

    writer.release();
    Python::attach(|py| {
        handler
            .call_method0(py, "close")
            .expect("close must not raise");
    });
}
