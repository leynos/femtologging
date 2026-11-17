//! Transport-origin coverage for the `tracing` bridge.
//!
//! `on_event` captures the marker before touching the event and suspends the
//! scope for the rest of the call. These tests pin the origin carried onto the
//! record and the suspended state observed by lazily rendered event fields.

use std::fmt;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use pyo3::Python;
use rstest::rstest;
use serial_test::serial;
use tracing_subscriber::prelude::*;

use super::FemtoTracingLayer;
use crate::handler::FemtoHandlerTrait;
use crate::http_handler::transport_origin::{enter_http_transport, is_http_transport_active};
use crate::log_record::{FemtoLogRecord, RecordOrigin};
use crate::manager;
use crate::test_utils::collecting_handler::CollectingHandler;

/// A debug-rendered field that records the transport state it observed.
struct ScopeProbe {
    /// Shared slot receiving the state seen while rendering.
    observed_active: Arc<AtomicBool>,
}

impl fmt::Debug for ScopeProbe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.observed_active
            .store(is_http_transport_active(), Ordering::SeqCst);
        f.write_str("probe")
    }
}

/// Reset the manager, attach a collector, and return it with its logger name.
fn collector_logger(logger_name: &str) -> Arc<CollectingHandler> {
    manager::reset_manager();
    let handler = Arc::new(CollectingHandler::default());
    Python::attach(|py| {
        let logger = manager::get_logger(py, logger_name).expect("logger created");
        logger
            .borrow(py)
            .add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>)
            .expect("collector registers");
    });
    handler
}

/// Flush `logger_name` and return the records the collector received.
fn drain(logger_name: &str, handler: &CollectingHandler) -> Vec<FemtoLogRecord> {
    Python::attach(|py| {
        let logger = manager::get_logger(py, logger_name).expect("logger created");
        assert!(logger.borrow(py).flush_handlers(), "flush should drain");
    });
    handler.collected()
}

/// An event emitted during transport work carries the marker.
#[rstest]
#[serial]
fn an_event_inside_transport_work_keeps_the_marker() {
    let logger_name = "tracing.origin.marked";
    let handler = collector_logger(logger_name);
    let subscriber = tracing_subscriber::registry().with(FemtoTracingLayer);

    let _scope = enter_http_transport();
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(target: "tracing.origin.marked", "transport diagnostic");
    });

    let records = drain(logger_name, &handler);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].origin(), RecordOrigin::SelfDiagnostic);
}

/// An ordinary event remains application-origin.
#[rstest]
#[serial]
fn an_event_outside_transport_work_stays_application() {
    let logger_name = "tracing.origin.plain";
    let handler = collector_logger(logger_name);
    let subscriber = tracing_subscriber::registry().with(FemtoTracingLayer);

    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(target: "tracing.origin.plain", "application event");
    });

    let records = drain(logger_name, &handler);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].origin(), RecordOrigin::Application);
}

/// Event fields are captured with the transport scope suspended.
#[rstest]
#[serial]
fn event_fields_are_captured_while_the_scope_is_suspended() {
    let logger_name = "tracing.origin.probe";
    let handler = collector_logger(logger_name);
    let observed_active = Arc::new(AtomicBool::new(true));
    let probe = ScopeProbe {
        observed_active: Arc::clone(&observed_active),
    };
    let subscriber = tracing_subscriber::registry().with(FemtoTracingLayer);

    let scope = enter_http_transport();
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(target: "tracing.origin.probe", probe = ?probe, "probe event");
    });

    assert!(
        !observed_active.load(Ordering::SeqCst),
        "field capture must see the suspended scope"
    );
    assert!(
        is_http_transport_active(),
        "on_event must restore the marker after the call"
    );
    drop(scope);

    let records = drain(logger_name, &handler);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].origin(), RecordOrigin::SelfDiagnostic);
    assert_eq!(records[0].metadata().key_values["probe"], "probe");
}
