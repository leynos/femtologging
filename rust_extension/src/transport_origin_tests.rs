//! Tests for the HTTP transport-origin thread marker.
//!
//! The marker is load-bearing in two directions. A transport thread must keep
//! it for its whole lifetime, and any nested work that suspends it must
//! restore the previous state exactly. These tests pin both, including the
//! unwind path, because a leaked marker would suppress ordinary records from
//! the HTTP worker's thread long after the request that set it.

use std::panic::{AssertUnwindSafe, catch_unwind};

use rstest::rstest;

use super::{
    capture_record_origin, enter_http_transport, is_http_transport_active, suspend_http_transport,
};
use crate::log_record::RecordOrigin;

/// The thread starts unmarked, and leaving the scope restores that.
#[rstest]
fn entering_marks_the_thread_and_dropping_restores() {
    assert!(!is_http_transport_active());
    {
        let _scope = enter_http_transport();
        assert!(is_http_transport_active());
    }
    assert!(!is_http_transport_active());
}

/// Nested scopes restore whichever state was current when they began.
#[rstest]
fn nested_enter_and_suspend_restore_in_last_in_first_out_order() {
    let outer = enter_http_transport();
    assert!(is_http_transport_active());

    let suspended = suspend_http_transport();
    assert!(!is_http_transport_active());

    let inner = enter_http_transport();
    assert!(is_http_transport_active());

    drop(inner);
    assert!(
        !is_http_transport_active(),
        "the inner entry restored the suspended state"
    );

    drop(suspended);
    assert!(
        is_http_transport_active(),
        "suspending then restoring the outer scope re-marks the thread"
    );

    drop(outer);
    assert!(!is_http_transport_active());
}

/// A bridge entry captures the marker before suspending it.
#[rstest]
fn captured_origin_follows_the_marker() {
    assert_eq!(capture_record_origin(), RecordOrigin::Application);

    let scope = enter_http_transport();
    assert_eq!(capture_record_origin(), RecordOrigin::HttpTransport);

    let suspended = suspend_http_transport();
    assert_eq!(
        capture_record_origin(),
        RecordOrigin::Application,
        "a record produced during suspended nested work is an application record"
    );

    drop(suspended);
    assert_eq!(capture_record_origin(), RecordOrigin::HttpTransport);
    drop(scope);
}

/// An unwind through a marked scope does not leave the thread marked.
#[rstest]
fn a_panic_restores_the_previous_state() {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let _scope = enter_http_transport();
        assert!(is_http_transport_active());
        panic!("unwind through the transport scope");
    }));

    assert!(result.is_err(), "the panic must reach the catch");
    assert!(
        !is_http_transport_active(),
        "the guard's drop must restore the state during unwinding"
    );
}

/// Another thread cannot observe or clear this thread's marker.
#[rstest]
fn the_marker_is_thread_local() {
    let scope = enter_http_transport();
    assert!(is_http_transport_active());

    let observed = std::thread::spawn(is_http_transport_active)
        .join()
        .expect("the probe thread should finish");
    assert!(!observed, "a new thread starts unmarked");

    assert!(
        is_http_transport_active(),
        "the probe thread must not change this thread's state"
    );
    drop(scope);
}
