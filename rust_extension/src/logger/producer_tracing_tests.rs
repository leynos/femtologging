//! Producer and worker tracing tests for bounded context-dispatch events.

use std::any::Any;
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, ThreadId};
use std::time::Duration;

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyAnyMethods, PyModule};
use serial_test::serial;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;

use super::context_snapshot::{ContextSnapshotProvider, ContextVarsSnapshotProvider};
use super::{FemtoLogger, HandlerAttachment};
use crate::filters::python_callback::PythonCallbackFilter;
use crate::filters::{FemtoFilter, FilterContext, FilterDecision};
use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::log_record::FemtoLogRecord;

#[path = "producer_tracing_test_assertions.rs"]
mod assertions;
#[path = "flush_tracing_tests.rs"]
mod flush_tracing_tests;
#[path = "producer_tracing_test_helpers.rs"]
mod helpers;
use helpers::{
    CapturedEvent, EXPECTED_EVENTS, PythonHandlerOutcome, assert_events, assert_filter_events,
    collect_fields, emit_capture_failure, emit_python_handler_dispatch,
    emit_real_handler_filter_dispatches, emit_rejected_native_dispatch,
};

/// Collects bounded internal tracing events from explicitly permitted threads.
#[derive(Clone)]
struct InternalEventCollector {
    /// Captured events retained for assertions.
    events: Arc<Mutex<Vec<CapturedEvent>>>,
    /// Threads currently allowed to contribute captured events.
    permitted_threads: Arc<Mutex<Vec<ThreadId>>>,
    /// Notifies waiters when a matching event is recorded.
    events_changed: Arc<Condvar>,
    /// Test thread retained so its permission is not revoked after an event.
    test_thread: ThreadId,
    /// Stops collection once the test has captured its expected outcomes.
    is_recording: Arc<AtomicBool>,
    /// Temporarily permits worker threads during a flush trace assertion.
    allow_all_threads: Arc<AtomicBool>,
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
            allow_all_threads: Arc::new(AtomicBool::new(false)),
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

    /// Capture events from the logger worker as well as the test thread.
    fn set_allow_all_threads(&self, allow: bool) {
        self.allow_all_threads.store(allow, Ordering::Relaxed);
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

        let is_permitted = self.allow_all_threads.load(Ordering::Relaxed)
            || match self.permitted_threads.lock() {
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

/// Context provider fixture that can return a controlled Python exception.
struct TestSnapshotProvider {
    /// Makes snapshot capture fail instead of returning Python `None`.
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

/// Python-backed handler fixture that emits internal events and can fail.
struct PythonTestHandler {
    /// Collector used to whitelist the worker thread and observe event output.
    collector: InternalEventCollector,
    /// Makes record handling return a controlled handler error.
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

    fn flush_until(&self, _deadline: std::time::Instant) -> bool {
        true
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

/// Native handler fixture with an optional controlled failure.
struct NativeTestHandler {
    /// Makes record handling return a controlled handler error.
    fail: bool,
}

/// Test filter that supplies a predetermined accept or reject outcome.
struct FixedDecisionFilter {
    /// Decision returned unchanged for each record presented to the fixture.
    accepted: bool,
}

impl FemtoFilter for FixedDecisionFilter {
    fn decision(
        &self,
        _record: &mut FemtoLogRecord,
        _context: &mut FilterContext,
    ) -> FilterDecision {
        FilterDecision::accept(self.accepted)
    }
}

impl FemtoHandlerTrait for NativeTestHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        if self.fail {
            Err(HandlerError::Message("test handler failure".to_owned()))
        } else {
            Ok(())
        }
    }

    fn flush_until(&self, _deadline: std::time::Instant) -> bool {
        true
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

fn emit_handler_filter_chain(filter: Arc<dyn FemtoFilter>, expected_acceptance: bool) {
    let attachment =
        HandlerAttachment::with_filters(Arc::new(NativeTestHandler { fail: false }), vec![filter]);
    let mut record = FemtoLogRecord::new(
        "trace-filter",
        crate::level::FemtoLevel::Info,
        "payload-is-not-traced",
    );
    let accepted = FemtoLogger::filter_handlers_for_test(&mut record, vec![attachment]);
    assert_eq!(accepted.len(), usize::from(expected_acceptance));
}

fn raising_callback_filter() -> Arc<dyn FemtoFilter> {
    Python::attach(|py| {
        let module = PyModule::from_code(
            py,
            CString::new(concat!(
                "def fail(record):\n",
                "    raise RuntimeError('payload-is-not-traced')\n",
            ))
            .expect("valid callback source")
            .as_c_str(),
            CString::new("test_filter_trace.py")
                .expect("valid filename")
                .as_c_str(),
            CString::new("test_filter_trace")
                .expect("valid module name")
                .as_c_str(),
        )
        .expect("callback module should load");
        Arc::new(PythonCallbackFilter::new(
            module
                .getattr("fail")
                .expect("raising callback should exist")
                .unbind(),
            "test_filter_trace.fail".to_owned(),
        )) as Arc<dyn FemtoFilter>
    })
}

fn emit_handler_filter_outcomes() {
    emit_handler_filter_chain(Arc::new(FixedDecisionFilter { accepted: true }), true);
    emit_handler_filter_chain(Arc::new(FixedDecisionFilter { accepted: false }), false);
    emit_handler_filter_chain(raising_callback_filter(), false);
}

/// Internal tracing events cover each dispatch outcome without exposing data.
#[test]
#[serial]
fn context_dispatch_tracing_reports_bounded_outcomes() {
    let collector = InternalEventCollector::new();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    tracing::subscriber::set_global_default(subscriber)
        .expect("test should install the global tracing collector");

    emit_python_handler_dispatch(&collector, PythonHandlerOutcome::Success)
        .expect("successful Python dispatch should emit its bounded events");
    emit_capture_failure(&collector).expect("capture failure should emit its bounded event");
    emit_rejected_native_dispatch()
        .expect("rejected native dispatch should emit its bounded events");
    emit_handler_filter_outcomes();
    emit_real_handler_filter_dispatches()
        .expect("real filter dispatch and propagation should emit bounded events");
    emit_python_handler_dispatch(&collector, PythonHandlerOutcome::Failure)
        .expect("failing Python handler should emit its bounded events");
    let events = collector
        .captured()
        .expect("tracing events should be captured without mutex poisoning");
    assert_eq!(events.len(), 23, "captured events: {events:?}");
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
    assert_events(&events, &EXPECTED_EVENTS);
    assert_filter_events(&events);
    assertions::assert_no_local_handler_events(&events);
    assertions::assert_no_handlers_accepted_events(&events);

    flush_tracing_tests::assert_unconfirmed_flush_trace(&collector);
}
