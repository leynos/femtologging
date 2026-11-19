//! Tests for the HTTP transport-origin thread marker.
//!
//! The marker is load-bearing in two directions. A transport thread must keep
//! it for its whole lifetime, and any nested work that suspends it must
//! restore the previous state exactly. These tests pin both, including the
//! unwind path, because a leaked marker would suppress ordinary records from
//! the HTTP worker's thread long after the request that set it.

use std::panic::{AssertUnwindSafe, catch_unwind};

use proptest::prelude::*;
use rstest::rstest;

use super::{
    TransportScope, capture_record_origin, enter_http_transport, is_http_transport_active,
    suspend_http_transport,
};
use crate::log_record::RecordOrigin;

/// The guard must not leave the OS thread whose state it restores.
///
/// `TransportScope` is crate-private, so an external `trybuild` fixture cannot
/// name it. This checks the same guarantee from inside the crate: the lookups
/// below are only unambiguous while the guard is neither `Send` nor `Sync`, so
/// reinstating either bound fails the build rather than a test run.
const _: fn() = || {
    /// Resolves only one way for a type that is not `Send`.
    trait AmbiguousIfSend<Marker> {
        /// Present only to create the candidate impls.
        fn probe() {}
    }
    impl<T: ?Sized> AmbiguousIfSend<()> for T {}
    impl<T: ?Sized + Send> AmbiguousIfSend<u8> for T {}

    /// Resolves only one way for a type that is not `Sync`.
    trait AmbiguousIfSync<Marker> {
        /// Present only to create the candidate impls.
        fn probe() {}
    }
    impl<T: ?Sized> AmbiguousIfSync<()> for T {}
    impl<T: ?Sized + Sync> AmbiguousIfSync<u8> for T {}

    let _ = <TransportScope as AmbiguousIfSend<_>>::probe;
    let _ = <TransportScope as AmbiguousIfSync<_>>::probe;
};

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
    assert_eq!(capture_record_origin(), RecordOrigin::SelfDiagnostic);

    let suspended = suspend_http_transport();
    assert_eq!(
        capture_record_origin(),
        RecordOrigin::Application,
        "a record produced during suspended nested work is an application record"
    );

    drop(suspended);
    assert_eq!(capture_record_origin(), RecordOrigin::SelfDiagnostic);
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

/// One operation in an arbitrary scope-nesting sequence.
#[derive(Clone, Copy, Debug)]
enum ScopeStep {
    /// Enter the transport state, as a worker or bridge entry does.
    Enter,
    /// Suspend the transport state, as a bridge does for nested work.
    Suspend,
    /// End the most recent scope, if any.
    End,
}

/// Arbitrary sequences of enter, suspend and end, including over-ending ones.
fn scope_steps() -> impl Strategy<Value = Vec<ScopeStep>> {
    prop::collection::vec(
        prop_oneof![
            Just(ScopeStep::Enter),
            Just(ScopeStep::Suspend),
            Just(ScopeStep::End),
        ],
        0..24,
    )
}

/// Apply one step, keeping the state it installed beside its guard.
fn apply_step(live: &mut Vec<(TransportScope, bool)>, step: ScopeStep) {
    match step {
        ScopeStep::Enter => live.push((enter_http_transport(), true)),
        ScopeStep::Suspend => live.push((suspend_http_transport(), false)),
        ScopeStep::End => {
            live.pop();
        }
    }
}

/// Check the marker against the innermost scope that is still alive.
fn assert_marker_matches(live: &[(TransportScope, bool)]) {
    let expected = live.last().is_some_and(|(_, marked)| *marked);
    assert_eq!(
        is_http_transport_active(),
        expected,
        "the marker must equal the state the innermost live scope installed"
    );
}

proptest! {
    /// The marker tracks the innermost live scope for any operation order.
    ///
    /// The fixed cases above pin the shapes a reader is likely to write; this
    /// covers the orderings and lengths nobody thought to write down.
    #[test]
    fn any_scope_sequence_tracks_the_innermost_live_scope(steps in scope_steps()) {
        let mut live = Vec::new();
        for step in steps {
            apply_step(&mut live, step);
            assert_marker_matches(&live);
        }
        while let Some((guard, _)) = live.pop() {
            drop(guard);
        }
        assert!(
            !is_http_transport_active(),
            "ending every scope in last-in-first-out order clears the marker"
        );
    }
}
