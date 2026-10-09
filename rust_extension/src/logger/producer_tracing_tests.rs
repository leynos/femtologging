//! Producer and worker tracing tests for bounded context-dispatch events.

use std::any::Any;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, ThreadId};
use std::time::Duration;

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::PyAnyMethods;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;

use super::context_snapshot::{ContextSnapshotProvider, ContextVarsSnapshotProvider};
use super::{FemtoLogger, QueuedRecord};
use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;

#[path = "producer_tracing_test_helpers.rs"]
mod helpers;
use helpers::{CapturedEvent, assert_events, collect_fields};

#[derive(Clone)]
struct InternalEventCollector {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
    permitted_threads: Arc<Mutex<Vec<ThreadId>>>,
    events_changed: Arc<Condvar>,
    test_thread: ThreadId,
    is_recording: Arc<AtomicBool>,
}

impl InternalEventCollector {
    fn new() -> Self {
        let test_thread = thread::current().id();
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
            permitted_threads: Arc::new(Mutex::new(vec![test_thread])),
            events_changed: Arc::new(Condvar::new()),
            test_thread,
            is_recording: Arc::new(AtomicBool::new(true)),
        }
    }

    fn captured(&self) -> Result<Vec<CapturedEvent>, String> {
        self.events
            .lock()
            .map(|events| events.clone())
            .map_err(|error| format!("tracing event collector was poisoned: {error}"))
    }

    fn stop_recording(&self) {
        self.is_recording.store(false, Ordering::Relaxed);
    }

    fn allow_current_thread(&self) -> Result<(), String> {
        let current_thread = thread::current().id();
        let mut permitted_threads = self
            .permitted_threads
            .lock()
            .map_err(|error| format!("permitted thread set was poisoned: {error}"))?;
        if !permitted_threads.contains(&current_thread) {
            permitted_threads.push(current_thread);
        }
        Ok(())
    }

    fn wait_for_event_count(&self, expected_count: usize) -> Result<(), String> {
        let events = self
            .events
            .lock()
            .map_err(|error| format!("tracing event collector was poisoned: {error}"))?;
        let (events, _) = self
            .events_changed
            .wait_timeout_while(events, Duration::from_secs(2), |events| {
                events.len() < expected_count
            })
            .map_err(|error| format!("tracing event collector was poisoned: {error}"))?;
        if events.len() >= expected_count {
            Ok(())
        } else {
            Err(format!(
                "timed out waiting for {expected_count} tracing events; observed {events:?}"
            ))
        }
    }
}

impl<S> Layer<S> for InternalEventCollector
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let current_thread = thread::current().id();
        if !self.is_recording.load(Ordering::Relaxed)
            || event.metadata().target() != "femtologging::internal"
        {
            return;
        }

        let is_permitted = match self.permitted_threads.lock() {
            Ok(permitted_threads) => permitted_threads.contains(&current_thread),
            Err(_) => return,
        };
        if !is_permitted {
            return;
        }

        let mut events = match self.events.lock() {
            Ok(events) => events,
            Err(_) => return,
        };
        events.push(CapturedEvent {
            fields: collect_fields(event),
        });
        drop(events);
        self.events_changed.notify_all();
        if current_thread == self.test_thread {
            return;
        }
        if let Ok(mut permitted_threads) = self.permitted_threads.lock() {
            permitted_threads.retain(|permitted_thread| *permitted_thread != current_thread);
        }
    }
}

struct TestSnapshotProvider {
    fail: bool,
}

impl ContextSnapshotProvider for TestSnapshotProvider {
    fn capture(&self) -> PyResult<Py<PyAny>> {
        if self.fail {
            Err(PyRuntimeError::new_err("test context capture failure"))
        } else {
            Python::attach(|py| Ok(py.None()))
        }
    }
}

struct PythonTestHandler {
    collector: InternalEventCollector,
    fail: bool,
}

impl FemtoHandlerTrait for PythonTestHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        self.collector
            .allow_current_thread()
            .map_err(HandlerError::Message)?;
        if self.fail {
            Err(HandlerError::Message("test handler failure".to_owned()))
        } else {
            Ok(())
        }
    }

    fn is_python_backed(&self) -> bool {
        true
    }

    fn provides_context_dispatch(&self) -> bool {
        true
    }

    fn handle_with_context(
        &self,
        record: FemtoLogRecord,
        _context: Option<&Py<PyAny>>,
    ) -> Result<(), HandlerError> {
        self.handle(record)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct NativeTestHandler {
    fail: bool,
}

impl FemtoHandlerTrait for NativeTestHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        if self.fail {
            Err(HandlerError::Message("test handler failure".to_owned()))
        } else {
            Ok(())
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn python_logger(
    provider: Arc<dyn ContextSnapshotProvider>,
    collector: InternalEventCollector,
    fail: bool,
) -> Result<FemtoLogger, HandlerError> {
    let logger =
        FemtoLogger::with_context_snapshot_provider("trace-test".to_owned(), None, provider);
    logger.add_handler(Arc::new(PythonTestHandler { collector, fail }))?;
    Ok(logger)
}

fn with_sensitive_context(log: impl FnOnce()) -> PyResult<()> {
    Python::attach(|py| -> PyResult<()> {
        let variable = py
            .import("contextvars")
            .and_then(|module| module.call_method1("ContextVar", ("trace_test_value",)))?;
        let token = variable.call_method1("set", ("context-secret-not-traced",))?;
        log();
        variable.call_method1("reset", (token,)).map(|_| ())
    })
}

/// Internal tracing events cover each dispatch outcome without exposing data.
#[test]
fn context_dispatch_tracing_reports_bounded_outcomes() {
    let collector = InternalEventCollector::new();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    tracing::subscriber::set_global_default(subscriber)
        .expect("test should install the global tracing collector");

    let successful_logger = python_logger(
        Arc::new(ContextVarsSnapshotProvider),
        collector.clone(),
        false,
    )
    .expect("context-aware Python test handler should register");
    with_sensitive_context(|| {
        assert!(
            successful_logger
                .log(FemtoLevel::Info, "message-secret-not-traced")
                .is_some()
        );
    })
    .expect("test ContextVar should be reset in its originating context");
    collector
        .wait_for_event_count(3)
        .expect("successful dispatch tracing events should arrive");
    drop(successful_logger);

    let failing_snapshot_logger = python_logger(
        Arc::new(TestSnapshotProvider { fail: true }),
        collector.clone(),
        false,
    )
    .expect("context-aware Python test handler should register");
    assert!(
        failing_snapshot_logger
            .log(FemtoLevel::Info, "message-secret-not-traced")
            .is_some()
    );
    drop(failing_snapshot_logger);

    let mut rejected_logger = FemtoLogger::new("trace-reject".to_owned());
    rejected_logger
        .add_handler(Arc::new(NativeTestHandler { fail: false }))
        .expect("native test handler should register");
    let (rejected_sender, rejected_receiver) = crate::sync::bounded::<QueuedRecord>(1);
    drop(rejected_receiver);
    rejected_logger.tx = Some(rejected_sender);
    assert!(
        rejected_logger
            .log(FemtoLevel::Info, "message-secret-not-traced")
            .is_some()
    );
    drop(rejected_logger);

    let failing_handler_logger = python_logger(
        Arc::new(ContextVarsSnapshotProvider),
        collector.clone(),
        true,
    )
    .expect("context-aware Python test handler should register");
    with_sensitive_context(|| {
        assert!(
            failing_handler_logger
                .log(FemtoLevel::Info, "message-secret-not-traced")
                .is_some()
        );
    })
    .expect("test ContextVar should be reset in its originating context");
    collector
        .wait_for_event_count(9)
        .expect("all context dispatch tracing events should arrive");
    drop(failing_handler_logger);
    collector.stop_recording();

    let events = collector
        .captured()
        .expect("tracing events should be captured without mutex poisoning");
    assert_eq!(events.len(), 9, "captured events: {events:?}");
    assert!(
        events
            .iter()
            .all(|event| event.fields.values().all(|value| {
                !value.contains("context-secret-not-traced")
                    && !value.contains("message-secret-not-traced")
                    && !value.contains("payload-is-not-traced")
            })),
        "event values must not contain context or record payloads: {events:?}"
    );
    assert_events(
        &events,
        &[
            (
                "producer_context_capture",
                "python",
                "capture_outcome",
                "captured",
                2,
            ),
            (
                "producer_queue_dispatch",
                "python",
                "dispatch_outcome",
                "queued",
                2,
            ),
            (
                "producer_context_capture",
                "python",
                "capture_outcome",
                "failed",
                1,
            ),
            (
                "producer_context_capture",
                "native",
                "capture_outcome",
                "not_required",
                1,
            ),
            (
                "producer_queue_dispatch",
                "native",
                "dispatch_outcome",
                "rejected",
                1,
            ),
            (
                "worker_handler_dispatch",
                "python",
                "dispatch_outcome",
                "success",
                1,
            ),
            (
                "worker_handler_dispatch",
                "python",
                "dispatch_outcome",
                "failed",
                1,
            ),
        ],
    );
}
