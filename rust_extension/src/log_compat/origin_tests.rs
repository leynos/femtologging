//! Transport-origin coverage for the Rust `log` bridge.
//!
//! The bridge captures the HTTP transport marker once, at entry, and suspends
//! it before running any user-visible work. These tests pin the captured
//! origin on dispatched records, the suspension seen by lazily formatted
//! arguments, and the two root-fallback routes that decide which logger
//! receives a marked record.

use std::fmt;
#[cfg(not(loom))]
use std::sync::Once;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[cfg(not(loom))]
use log::LevelFilter;
use log::Log;
use pyo3::prelude::*;
use rstest::rstest;
use serial_test::serial;

use super::FemtoLogAdapter;
use super::test_support::{log_max_level, unique_logger_name};
use crate::handler::FemtoHandlerTrait;
use crate::http_handler::transport_origin::{
    enter_http_transport, is_http_transport_active, suspend_http_transport,
};
use crate::log_record::{FemtoLogRecord, RecordOrigin};
use crate::manager;
#[cfg(not(loom))]
use crate::manager::MAX_LOGGERS;
use crate::test_utils::collecting_handler::CollectingHandler;

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

/// One marked-probe route through the bridge.
struct MarkedRoute<'a> {
    /// Logger the probe is emitted to.
    target: &'a str,
    /// Logger whose collector must receive the probe.
    sink: &'a str,
    /// Logger name the collected record must retain.
    expected_logger: &'a str,
}

/// Emit one marked probe along `route` and assert its single delivery.
///
/// The sink differs from the target on fallback and propagation paths, which
/// is what lets one helper cover both without repeating the assertion block.
fn assert_marked_route(route: MarkedRoute<'_>) {
    let handler = attach_collector(route.sink);
    let _scope = enter_http_transport();
    log_info_record(route.target);
    let records = flushed_records(route.sink, &handler);
    assert_single_record(&records, RecordOrigin::SelfDiagnostic);
    assert_eq!(records[0].logger(), route.expected_logger);
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
    assert_marked_route(MarkedRoute {
        target: &unique_logger_name,
        sink: &unique_logger_name,
        expected_logger: &unique_logger_name,
    });
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
    assert_single_record(&records, RecordOrigin::SelfDiagnostic);
}

/// An invalid target falls back to root and keeps the marker.
#[rstest]
#[serial]
fn an_invalid_target_falls_back_to_root_with_the_marker(_log_max_level: ()) {
    Python::attach(|_| manager::reset_manager());
    assert_marked_route(MarkedRoute {
        target: "invalid..target",
        sink: "root",
        expected_logger: "root",
    });
}

/// A full registry falls back to root and keeps the marker.
///
/// The registry is process-global and one test cannot un-register ten thousand
/// loggers, so the saturated path runs in a fresh process. Without the marker
/// surviving that fallback, a transport diagnostic routed to root would be
/// delivered by the very HTTP handler that produced it.
///
/// Subprocess tests are excluded from the Loom unit-test build: spawning this
/// binary under `--cfg loom` would re-enter the model build through a child
/// process that shares none of the model's state.
#[cfg(not(loom))]
#[test]
fn a_full_registry_falls_back_to_root_with_the_marker() {
    let output =
        crate::test_utils::isolated_child::run_isolated_test("log_bridge_origin_capacity_child")
            .expect("the isolated origin test should start");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "the isolated origin test must run and pass; status: {:?}\n\
         stdout:\n{stdout}\nstderr:\n{stderr}",
        output.status
    );
}

// The registry this child resets and saturates is process-global, so it must
// not overlap another ignored test that saturates it. The heavy workflow's
// ignored lanes run with default test threads.
#[cfg(not(loom))]
#[serial]
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
    });

    assert_marked_route(MarkedRoute {
        target: "origin-capacity-overflow",
        sink: "root",
        expected_logger: "root",
    });
}

/// Parent propagation copies the record without losing the marker.
#[rstest]
#[serial]
fn parent_propagation_preserves_the_marker(_log_max_level: ()) {
    Python::attach(|_| manager::reset_manager());
    assert_marked_route(MarkedRoute {
        target: "bridge.origin.propagation.child",
        sink: "bridge.origin.propagation",
        expected_logger: "bridge.origin.propagation.child",
    });
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
