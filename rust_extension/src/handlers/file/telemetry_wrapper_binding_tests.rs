//! Binding-level tests that the rotating wrappers label their own waits.
//!
//! The rotating and timed-rotating bindings delegate their work to the same
//! core file handler as the plain file binding, so only the `handler_kind`
//! label distinguishes them in the diagnostics. These tests call each wrapper
//! through its Python method names, read the public `wait_diagnostics()` map
//! and the internal trace event, and pin that label. They live beside the
//! shared event collector in the file module because that collector is scoped
//! to `handlers::file`.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::mpsc;

use pyo3::{Py, PyAny, Python};
use rstest::rstest;
use tempfile::TempDir;
use tracing_subscriber::prelude::*;

use super::scripted_writer::{ScriptedWriter, Stage};
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
/// own call, so the writer only reports stages that call caused.
fn scripted_config(writer: ScriptedWriter) -> TestConfig<ScriptedWriter, DefaultFormatter> {
    let mut config = TestConfig::new(writer, DefaultFormatter);
    config.capacity = 1;
    config.flush_interval = 0;
    config.overflow_policy = OverflowPolicy::Block;
    config
}

/// Build a rotating wrapper over a scripted writer.
fn rotating_over_writer(py: Python<'_>, writer: ScriptedWriter) -> Py<PyAny> {
    let core = FemtoRotatingFileHandler::with_writer_for_test(scripted_config(writer));
    Py::new(py, PyRotatingFileHandler::from_core(core))
        .expect("the rotating handler must be a valid pyclass")
        .into_any()
}

/// Build a timed-rotating wrapper over a scripted writer.
fn timed_rotating_over_writer(py: Python<'_>, writer: ScriptedWriter) -> Py<PyAny> {
    let inner = FemtoFileHandler::with_writer_for_test(scripted_config(writer));
    let schedule = TimedRotationSchedule::new(TimedRotationWhen::Hours, 1, true, None)
        .expect("an hourly schedule is valid");
    let core = FemtoTimedRotatingFileHandler::new_with_schedule(inner, schedule, 2);
    Py::new(py, PyTimedRotatingFileHandler::from_core(core))
        .expect("the timed-rotating handler must be a valid pyclass")
        .into_any()
}

/// How the scripted worker fails, and the outcome the wait must record.
#[derive(Clone, Copy, Debug)]
enum Failure {
    /// A worker whose flush reports an I/O error.
    FlushFails,
    /// A worker that panics inside its terminal flush.
    ClosePanics,
}

impl Failure {
    /// Build the writer that plays out this failure.
    fn writer(self, entered: mpsc::Sender<Stage>) -> ScriptedWriter {
        match self {
            Self::FlushFails => ScriptedWriter::failing_flush(entered),
            Self::ClosePanics => ScriptedWriter::panicking_flush(entered),
        }
    }

    /// The operation the failing wait belongs to.
    const fn operation(self) -> &'static str {
        match self {
            Self::FlushFails => "handler_flush",
            Self::ClosePanics => "handler_close",
        }
    }

    /// The counter key and event outcome the failing wait must record.
    const fn outcome(self) -> &'static str {
        match self {
            Self::FlushFails => "flush_failed",
            Self::ClosePanics => "close_panicked",
        }
    }
}

/// One wrapper failure case: the wrapper, its expected kind, and the failure.
struct FailureCase {
    /// Builds the wrapper under test over the case's scripted writer.
    make_wrapper: fn(Python<'_>, ScriptedWriter) -> Py<PyAny>,
    /// The `handler_kind` every event must carry.
    expected_kind: &'static str,
    /// The failure the worker plays out.
    failure: Failure,
}

/// Read the public diagnostics map for a wrapper.
fn diagnostics_map(py: Python<'_>, handler: &Py<PyAny>) -> BTreeMap<String, u64> {
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

/// A wrapper's failing wait is counted and labelled with its own kind.
///
/// The delegating wrappers share the core file handler's failure paths, so a
/// regression in the shared binding would show up on the plain file handler
/// first. These cases pin that each wrapper still records the failure against
/// its own `handler_kind`, which is the only label that tells an operator which
/// class produced the wait.
#[rstest]
#[case::rotating_flush_failed(FailureCase {
    make_wrapper: rotating_over_writer,
    expected_kind: "rotating",
    failure: Failure::FlushFails,
})]
#[case::rotating_close_panicked(FailureCase {
    make_wrapper: rotating_over_writer,
    expected_kind: "rotating",
    failure: Failure::ClosePanics,
})]
#[case::timed_rotating_flush_failed(FailureCase {
    make_wrapper: timed_rotating_over_writer,
    expected_kind: "timed_rotating",
    failure: Failure::FlushFails,
})]
#[case::timed_rotating_close_panicked(FailureCase {
    make_wrapper: timed_rotating_over_writer,
    expected_kind: "timed_rotating",
    failure: Failure::ClosePanics,
})]
fn each_wrapper_labels_its_failure_waits(#[case] case: FailureCase) {
    let (entered_tx, _entered_rx) = mpsc::channel();
    let writer = case.failure.writer(entered_tx);

    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let (diagnostics, handler) = tracing::subscriber::with_default(subscriber, || {
        Python::attach(|py| {
            let handler = (case.make_wrapper)(py, writer.clone());
            match case.failure {
                Failure::FlushFails => {
                    let flushed: bool = handler
                        .call_method0(py, "flush")
                        .expect("flush must not raise")
                        .extract(py)
                        .expect("flush must return a bool");
                    assert!(
                        !flushed,
                        "a flush the worker could not complete must return False",
                    );
                }
                Failure::ClosePanics => {
                    handler
                        .call_method0(py, "close")
                        .expect("a panicking worker must not raise from close()");
                }
            }
            (diagnostics_map(py, &handler), handler)
        })
    });

    // A flush failure leaves the handler open, so close it to keep no worker
    // thread behind. The call runs outside the subscriber so its own event
    // cannot be mistaken for the failing flush's.
    writer.release();
    if matches!(case.failure, Failure::FlushFails) {
        Python::attach(|py| {
            handler
                .call_method0(py, "close")
                .expect("the later close must not raise");
        });
    }

    assert_eq!(
        diagnostics[case.failure.outcome()],
        1,
        "the wrapper must count its {}: {diagnostics:?}",
        case.failure.outcome(),
    );
    assert_eq!(
        diagnostics["waits_in_flight"], 0,
        "a finished wait must leave the gauge: {diagnostics:?}",
    );

    let events = collector.captured();
    assert_eq!(events.len(), 1, "one wait must emit one event: {events:?}");
    assert_eq!(
        events[0].fields["handler_kind"], case.expected_kind,
        "the failure must carry the wrapper's own kind: {events:?}",
    );
    assert_eq!(events[0].fields["operation"], case.failure.operation());
    assert_eq!(events[0].fields["wait_outcome"], case.failure.outcome());
    assert_eq!(events[0].fields["policy_bucket"], "block");
}
