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
use super::telemetry_event_tests::EventCollector;
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
fn test_config() -> HandlerConfig {
    HandlerConfig {
        capacity: 4,
        flush_interval: 1,
        overflow_policy: OverflowPolicy::Block,
    }
}

/// Queue configuration for a wrapper built over a scripted writer.
///
/// Capacity one and no periodic flush keep the worker idle until the case's
/// own call, so the writer only reports stages that call caused. The policy
/// comes from the case, because an enqueue timeout and an enqueue refusal are
/// different outcomes of different policies.
pub(super) fn scripted_config(
    writer: ScriptedWriter,
    policy: OverflowPolicy,
) -> TestConfig<ScriptedWriter, DefaultFormatter> {
    let mut config = TestConfig::new(writer, DefaultFormatter);
    config.capacity = 1;
    config.flush_interval = 0;
    config.overflow_policy = policy;
    config
}

/// Build a rotating wrapper over a scripted writer.
pub(super) fn rotating_over_writer(
    py: Python<'_>,
    writer: ScriptedWriter,
    policy: OverflowPolicy,
) -> Py<PyAny> {
    let core = FemtoRotatingFileHandler::with_writer_for_test(scripted_config(writer, policy));
    Py::new(py, PyRotatingFileHandler::from_core(core))
        .expect("the rotating handler must be a valid pyclass")
        .into_any()
}

/// Build a timed-rotating wrapper over a scripted writer.
pub(super) fn timed_rotating_over_writer(
    py: Python<'_>,
    writer: ScriptedWriter,
    policy: OverflowPolicy,
) -> Py<PyAny> {
    let inner = FemtoFileHandler::with_writer_for_test(scripted_config(writer, policy));
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

    assert_eq!(
        diagnostics["handle_accepted"], 1,
        "the wrapper must count its accepted record: {diagnostics:?}",
    );
    assert_eq!(
        diagnostics["flush_acknowledged"], 1,
        "the wrapper must count its acknowledged flush: {diagnostics:?}",
    );
    assert_eq!(
        diagnostics["close_completed"], 1,
        "the wrapper must count its shutdown: {diagnostics:?}",
    );

    let events = collector.captured();
    let kinds: Vec<_> = events
        .iter()
        .map(|event| event.fields["handler_kind"].clone())
        .collect();
    assert_eq!(
        kinds,
        [expected_kind, expected_kind, expected_kind],
        "every wait must carry the wrapper's own kind: {events:?}",
    );
    let operations: Vec<_> = events
        .iter()
        .map(|event| event.fields["operation"].clone())
        .collect();
    assert_eq!(
        operations,
        ["handler_handle", "handler_flush", "handler_close"],
        "one wait must emit one event: {events:?}",
    );
    let outcomes: Vec<_> = events
        .iter()
        .map(|event| event.fields["wait_outcome"].clone())
        .collect();
    assert_eq!(
        outcomes,
        ["handle_accepted", "flush_acknowledged", "close_completed"],
        "the wrappers must record the outcomes they observed: {events:?}",
    );
}
