//! Binding-level tests that the rotating wrappers label their own waits.
//!
//! The rotating and timed-rotating bindings delegate their work to the same
//! core file handler as the plain file binding, so only the `handler_kind`
//! label distinguishes them in the diagnostics. These tests call each wrapper
//! through its Python method names, read the public `wait_diagnostics()` map
//! and the internal trace event, and pin that label. They live beside the
//! shared event collector in the file module because that collector is scoped
//! to `handlers::file`. The failure paths live in
//! [`super::telemetry_wrapper_failure_tests`], which reuses the scripted
//! wrapper builders defined here.

use std::collections::BTreeMap;
use std::path::Path;

use pyo3::{Py, PyAny, Python};
use rstest::rstest;
use tempfile::TempDir;
use tracing_subscriber::prelude::*;

use super::scripted_writer::ScriptedWriter;
use super::telemetry_event_tests::{CapturedEvent, EventCollector};
use super::{FemtoFileHandler, HandlerConfig, OverflowPolicy, TestConfig};
use crate::formatter::DefaultFormatter;
use crate::handlers::rotating::{FemtoRotatingFileHandler, PyRotatingFileHandler, RotationConfig};
use crate::handlers::timed_rotating::{
    FemtoTimedRotatingFileHandler, PyTimedRotatingFileHandler, TimedRotationConfig,
    TimedRotationSchedule, TimedRotationWhen,
};

/// Build a rotating wrapper over a real file.
fn rotating_wrapper(py: Python<'_>, path: &Path) -> Py<PyAny> {
    let core = FemtoRotatingFileHandler::with_capacity_flush_policy(
        path,
        DefaultFormatter,
        test_config(),
        RotationConfig::new(1 << 20, 2),
    )
    .expect("the rotating handler must open its file");
    Py::new(py, PyRotatingFileHandler::from_core(core))
        .expect("the rotating handler must be a valid pyclass")
        .into_any()
}

/// Build a timed-rotating wrapper over a real file.
fn timed_rotating_wrapper(py: Python<'_>, path: &Path) -> Py<PyAny> {
    let schedule = TimedRotationSchedule::new(TimedRotationWhen::Hours, 1, true, None)
        .expect("an hourly schedule is valid");
    let core = FemtoTimedRotatingFileHandler::with_capacity_flush_policy(
        path,
        DefaultFormatter,
        test_config(),
        TimedRotationConfig {
            schedule,
            backup_count: 2,
        },
    )
    .expect("the timed-rotating handler must open its file");
    Py::new(py, PyTimedRotatingFileHandler::from_core(core))
        .expect("the timed-rotating handler must be a valid pyclass")
        .into_any()
}

/// Queue configuration shared by the wrapper cases.
const fn test_config() -> HandlerConfig {
    HandlerConfig {
        capacity: 4,
        flush_interval: 1,
        overflow_policy: OverflowPolicy::Block,
    }
}

/// Queue configuration for a wrapper built over a scripted writer.
///
/// No periodic flush keeps the worker idle until the case's own call, so the
/// writer only reports stages that call caused. The capacity and the policy
/// come from the case: a narrow queue is what makes a refusal or an enqueue
/// timeout reachable, and a wider one leaves room while the worker is parked.
pub(super) const fn scripted_config(
    writer: ScriptedWriter,
    policy: OverflowPolicy,
    capacity: usize,
) -> TestConfig<ScriptedWriter, DefaultFormatter> {
    let mut config = TestConfig::new(writer, DefaultFormatter);
    config.capacity = capacity;
    config.flush_interval = 0;
    config.overflow_policy = policy;
    config
}

/// Build a rotating wrapper over a scripted writer with capacity one.
pub(super) fn rotating_over_writer(
    py: Python<'_>,
    writer: ScriptedWriter,
    policy: OverflowPolicy,
) -> Py<PyAny> {
    rotating_over_writer_with_capacity(py, writer, policy, 1)
}

/// Build a rotating wrapper over a scripted writer with `capacity` slots.
pub(super) fn rotating_over_writer_with_capacity(
    py: Python<'_>,
    writer: ScriptedWriter,
    policy: OverflowPolicy,
    capacity: usize,
) -> Py<PyAny> {
    let core =
        FemtoRotatingFileHandler::with_writer_for_test(scripted_config(writer, policy, capacity));
    Py::new(py, PyRotatingFileHandler::from_core(core))
        .expect("the rotating handler must be a valid pyclass")
        .into_any()
}

/// Build a timed-rotating wrapper over a scripted writer with capacity one.
pub(super) fn timed_rotating_over_writer(
    py: Python<'_>,
    writer: ScriptedWriter,
    policy: OverflowPolicy,
) -> Py<PyAny> {
    timed_rotating_over_writer_with_capacity(py, writer, policy, 1)
}

/// Build a timed-rotating wrapper over a scripted writer with `capacity` slots.
pub(super) fn timed_rotating_over_writer_with_capacity(
    py: Python<'_>,
    writer: ScriptedWriter,
    policy: OverflowPolicy,
    capacity: usize,
) -> Py<PyAny> {
    let inner = FemtoFileHandler::with_writer_for_test(scripted_config(writer, policy, capacity));
    let schedule = TimedRotationSchedule::new(TimedRotationWhen::Hours, 1, true, None)
        .expect("an hourly schedule is valid");
    let core = FemtoTimedRotatingFileHandler::new_with_schedule(inner, schedule, 2);
    Py::new(py, PyTimedRotatingFileHandler::from_core(core))
        .expect("the timed-rotating handler must be a valid pyclass")
        .into_any()
}

/// Read the public diagnostics map for a wrapper.
pub(super) fn diagnostics_map(py: Python<'_>, handler: &Py<PyAny>) -> BTreeMap<String, u64> {
    handler
        .call_method0(py, "wait_diagnostics")
        .expect("wait_diagnostics must not raise")
        .extract(py)
        .expect("wait_diagnostics must return a counter map")
}

/// Each wrapper's successful waits are counted and labelled with its own kind.
#[rstest]
#[case::rotating(rotating_wrapper as fn(Python<'_>, &Path) -> Py<PyAny>, "rotating")]
#[case::timed_rotating(
    timed_rotating_wrapper as fn(Python<'_>, &Path) -> Py<PyAny>,
    "timed_rotating"
)]
fn each_wrapper_labels_its_own_waits(
    #[case] make_wrapper: fn(Python<'_>, &Path) -> Py<PyAny>,
    #[case] expected_kind: &'static str,
) {
    let (diagnostics, events) = drive_wrapper(make_wrapper);

    assert_wait_counts(&diagnostics);
    assert_wait_events(&events, expected_kind);
}

/// Drive one wrapper through handle, flush and close.
///
/// The returned diagnostics are read while the scoped subscriber is still
/// installed, and the events are read from the collector after it has seen
/// every wait the three calls cause.
fn drive_wrapper(
    make_wrapper: fn(Python<'_>, &Path) -> Py<PyAny>,
) -> (BTreeMap<String, u64>, Vec<CapturedEvent>) {
    let directory = TempDir::new().expect("a temporary directory");
    let path = directory.path().join("wrapper.log");

    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let diagnostics = tracing::subscriber::with_default(subscriber, || {
        Python::attach(|py| {
            let handler = make_wrapper(py, &path);
            handler
                .call_method1(py, "handle", ("core", "INFO", "hello"))
                .expect("the first record must be accepted");
            let flushed: bool = handler
                .call_method0(py, "flush")
                .expect("flush must not raise")
                .extract(py)
                .expect("flush must return a bool");
            assert!(flushed, "an idle worker must acknowledge the flush");
            handler
                .call_method0(py, "close")
                .expect("close must not raise");
            diagnostics_map(py, &handler)
        })
    });

    (diagnostics, collector.captured())
}

/// Check that the wrapper counted its own accepted record, flush and close.
fn assert_wait_counts(diagnostics: &BTreeMap<String, u64>) {
    assert_eq!(
        diagnostics.get("handle_accepted").copied(),
        Some(1),
        "the wrapper must count its accepted record: {diagnostics:?}",
    );
    assert_eq!(
        diagnostics.get("flush_acknowledged").copied(),
        Some(1),
        "the wrapper must count its acknowledged flush: {diagnostics:?}",
    );
    assert_eq!(
        diagnostics.get("close_completed").copied(),
        Some(1),
        "the wrapper must count its shutdown: {diagnostics:?}",
    );
}

/// Check that every wait carried the wrapper's own kind, that one wait emitted
/// one event, and that each event records the outcome the operation observed.
fn assert_wait_events(events: &[CapturedEvent], expected_kind: &str) {
    let kinds: Vec<_> = events
        .iter()
        .map(|event| event.fields.get("handler_kind").map(String::as_str))
        .collect();
    assert_eq!(
        kinds,
        [
            Some(expected_kind),
            Some(expected_kind),
            Some(expected_kind)
        ],
        "every wait must carry the wrapper's own kind: {events:?}",
    );
    let operations: Vec<_> = events
        .iter()
        .map(|event| event.fields.get("operation").map(String::as_str))
        .collect();
    assert_eq!(
        operations,
        [
            Some("handler_handle"),
            Some("handler_flush"),
            Some("handler_close")
        ],
        "one wait must emit one event: {events:?}",
    );
    let outcomes: Vec<_> = events
        .iter()
        .map(|event| event.fields.get("wait_outcome").map(String::as_str))
        .collect();
    assert_eq!(
        outcomes,
        [
            Some("handle_accepted"),
            Some("flush_acknowledged"),
            Some("close_completed")
        ],
        "the wrappers must record the outcomes they observed: {events:?}",
    );
}
