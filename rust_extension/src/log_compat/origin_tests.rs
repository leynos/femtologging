//! Transport-origin coverage for the Rust `log` bridge.
//!
//! The bridge captures the HTTP transport marker once, at entry, and suspends
//! it before running any user-visible work. These tests pin the captured
//! origin on dispatched records, the suspension seen by lazily formatted
//! arguments, and the two root-fallback routes that decide which logger
//! receives a marked record.

use std::fmt;
use std::sync::{
    Arc, Once,
    atomic::{AtomicBool, Ordering},
};

use log::{LevelFilter, Log};
use pyo3::prelude::*;
use rstest::rstest;
use serial_test::serial;

use super::FemtoLogAdapter;
use super::test_support::{log_max_level, unique_logger_name};
use crate::handler::FemtoHandlerTrait;
use crate::log_record::{FemtoLogRecord, RecordOrigin};
use crate::manager::{self, MAX_LOGGERS};
use crate::test_utils::collecting_handler::CollectingHandler;
use crate::transport_origin::{
    enter_http_transport, is_http_transport_active, suspend_http_transport,
};

/// A lazily formatted argument that records the transport state it observed.
struct ScopeProbe {
    /// Shared slot receiving the state seen while formatting.
    observed_active: Arc<AtomicBool>,
}

impl fmt::Display for ScopeProbe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.observed_active
            .store(is_http_transport_active(), Ordering::SeqCst);
        f.write_str("probe")
    }
}

/// Attach a collecting handler to `logger_name` and return it.
fn attach_collector(logger_name: &str) -> Arc<CollectingHandler> {
    Python::attach(|py| {
        let logger = manager::get_logger(py, logger_name).expect("logger created");
        let handler = Arc::new(CollectingHandler::default());
        logger
            .borrow(py)
            .add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>)
            .expect("native collecting handler should register");
        handler
    })
}

/// Message used by the bridge-origin probes.
///
/// The tests assert the record's origin and logger, not its text, so one
/// message keeps the probe helper to a single string argument.
const PROBE_MESSAGE: &str = "bridge origin probe";

/// Emit one information record for `target` through the adapter.
fn log_info_record(target: &str) {
    FemtoLogAdapter.log(
        &log::Record::builder()
            .args(format_args!("{PROBE_MESSAGE}"))
            .level(log::Level::Info)
            .target(target)
            .build(),
    );
}

/// Assert the collector received exactly one record with `origin`.
fn assert_single_record(records: &[FemtoLogRecord], origin: RecordOrigin) {
    assert_eq!(records.len(), 1, "expected exactly one record");
    assert_eq!(records[0].origin(), origin);
}

/// Flush `logger_name` and return the records its collector received.
fn flushed_records(logger_name: &str, handler: &CollectingHandler) -> Vec<FemtoLogRecord> {
    Python::attach(|py| {
        let logger = manager::get_logger(py, logger_name).expect("logger created");
        assert!(logger.borrow(py).flush_handlers(), "flush should drain");
    });
    handler.collected()
}

/// A record emitted inside transport work keeps the marker onto the record.
#[rstest]
#[serial]
fn adapter_marks_records_emitted_inside_transport_work(
    _log_max_level: (),
    unique_logger_name: String,
) {
    let handler = attach_collector(&unique_logger_name);

    let _scope = enter_http_transport();
    log_info_record(&unique_logger_name);

    let records = flushed_records(&unique_logger_name, &handler);
    assert_single_record(&records, RecordOrigin::HttpTransport);
}

/// A record emitted outside transport work remains application-origin.
#[rstest]
#[serial]
fn adapter_leaves_ordinary_records_unmarked(_log_max_level: (), unique_logger_name: String) {
    let handler = attach_collector(&unique_logger_name);

    log_info_record(&unique_logger_name);

    let records = flushed_records(&unique_logger_name, &handler);
    assert_single_record(&records, RecordOrigin::Application);
}

/// Arguments are formatted after the bridge suspends the transport scope.
///
/// A `Display` implementation that logs, or inspects the marker, during
/// argument formatting must not see the transport state: otherwise an
/// application callback invoked by the bridge could emit a record that the
/// HTTP handler then suppresses as if it were a transport diagnostic.
#[rstest]
#[serial]
fn lazily_formatted_arguments_observe_a_suspended_scope(
    _log_max_level: (),
    unique_logger_name: String,
) {
    let handler = attach_collector(&unique_logger_name);
    let observed_active = Arc::new(AtomicBool::new(true));
    let probe = ScopeProbe {
        observed_active: Arc::clone(&observed_active),
    };
    let scope = enter_http_transport();
    FemtoLogAdapter.log(
        &log::Record::builder()
            .args(format_args!("{probe}"))
            .level(log::Level::Info)
            .target(unique_logger_name.as_str())
            .build(),
    );

    assert!(
        !observed_active.load(Ordering::SeqCst),
        "the argument must be formatted with the scope suspended"
    );
    assert!(
        is_http_transport_active(),
        "the bridge must restore the marker after the call"
    );
    drop(scope);

    let records = flushed_records(&unique_logger_name, &handler);
    assert_single_record(&records, RecordOrigin::HttpTransport);
}

/// An invalid target falls back to root and keeps the marker.
#[rstest]
#[serial]
fn an_invalid_target_falls_back_to_root_with_the_marker(_log_max_level: ()) {
    Python::attach(|_| manager::reset_manager());
    let handler = attach_collector("root");

    let _scope = enter_http_transport();
    log_info_record("invalid..target");

    let records = flushed_records("root", &handler);
    assert_single_record(&records, RecordOrigin::HttpTransport);
    assert_eq!(records[0].logger(), "root");
}

/// A full registry falls back to root and keeps the marker.
///
/// The registry is process-global and one test cannot un-register ten thousand
/// loggers, so the saturated path runs in a fresh process. Without the marker
/// surviving that fallback, a transport diagnostic routed to root would be
/// delivered by the very HTTP handler that produced it.
#[test]
fn a_full_registry_falls_back_to_root_with_the_marker() {
    let output =
        crate::test_utils::isolated_child::run_isolated_test("log_bridge_origin_capacity_child")
            .expect("the isolated origin test should start");
    assert!(
        output.status.success(),
        "isolated origin test failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
#[ignore = "run in a fresh process because the manager is process-global"]
fn log_bridge_origin_capacity_child() {
    static INIT: Once = Once::new();
    INIT.call_once(|| log::set_max_level(LevelFilter::Trace));

    Python::attach(|py| {
        manager::reset_manager();
        for index in 0..MAX_LOGGERS {
            manager::get_logger(py, &format!("origin-capacity-{index}"))
                .expect("saturating logger should register");
        }
        let root = manager::get_logger(py, "root").expect("root exists");
        let handler = Arc::new(CollectingHandler::default());
        root.borrow(py)
            .add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>)
            .expect("collector registers on root");

        let _scope = enter_http_transport();
        log_info_record("origin-capacity-overflow");

        assert!(root.borrow(py).flush_handlers(), "root should drain");
        let records = handler.collected();
        assert_single_record(&records, RecordOrigin::HttpTransport);
        assert_eq!(records[0].logger(), "root");
    });
}

/// Parent propagation copies the record without losing the marker.
#[rstest]
#[serial]
fn parent_propagation_preserves_the_marker(_log_max_level: ()) {
    Python::attach(|_| manager::reset_manager());
    let parent_name = "bridge.origin.propagation";
    let child_name = "bridge.origin.propagation.child";
    let handler = attach_collector(parent_name);

    let _scope = enter_http_transport();
    log_info_record(child_name);

    let records = flushed_records(parent_name, &handler);
    assert_single_record(&records, RecordOrigin::HttpTransport);
    assert_eq!(records[0].logger(), child_name);
}

/// Suspending is visible to the bridge's own nested call, but not outside it.
#[rstest]
fn a_suspended_scope_is_restored_by_its_guard() {
    let outer = enter_http_transport();
    {
        let _suspended = suspend_http_transport();
        assert!(!is_http_transport_active());
    }
    assert!(is_http_transport_active());
    drop(outer);
}
