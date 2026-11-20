//! Completion-broadcast tests.

use std::{panic::AssertUnwindSafe, sync::Arc, time::Duration};

use super::{CompleteOnDrop, ShutdownCompletion};

/// Every waiter registered before completion is woken.
#[test]
fn every_registered_waiter_observes_completion() {
    let completion = ShutdownCompletion::new();
    let first = completion.register();
    let second = completion.register();

    completion.complete();

    for waiter in [first, second] {
        waiter
            .recv_timeout(Duration::from_secs(1))
            .expect("a registered waiter must be woken by completion");
    }
}

/// A waiter that arrives after completion does not wait for the timeout.
#[test]
fn a_late_waiter_returns_immediately() {
    let completion = ShutdownCompletion::new();
    completion.complete();

    assert!(
        completion.wait(Duration::from_secs(30)),
        "a completed worker must satisfy a late waiter at once",
    );
}

/// An uncompleted worker leaves a waiter to time out.
#[test]
fn an_uncompleted_worker_times_out() {
    let completion = ShutdownCompletion::new();

    assert!(
        !completion.wait(Duration::from_millis(1)),
        "an unfinished worker must not report completion",
    );
}

/// Completion is idempotent and keeps waking later waiters.
#[test]
fn completion_can_be_signalled_twice() {
    let completion = ShutdownCompletion::new();
    completion.complete();
    completion.complete();

    assert!(
        completion.wait(Duration::from_secs(1)),
        "a repeated completion must still satisfy a later waiter",
    );
}

/// The guard completes when its scope ends normally.
#[test]
fn the_drop_guard_completes_on_a_normal_exit() {
    let completion = ShutdownCompletion::new();
    {
        let _signal = CompleteOnDrop::new(Arc::clone(&completion));
    }

    assert!(
        completion.wait(Duration::from_millis(1)),
        "leaving the guarded scope must signal completion",
    );
}

/// The guard completes while a panicking worker unwinds.
#[test]
fn the_drop_guard_completes_while_unwinding() {
    let completion = ShutdownCompletion::new();
    let worker_completion = Arc::clone(&completion);
    let outcome = std::panic::catch_unwind(AssertUnwindSafe(move || {
        let _signal = CompleteOnDrop::new(worker_completion);
        panic!("simulated worker failure");
    }));

    assert!(outcome.is_err(), "the simulated worker must have panicked");
    assert!(
        completion.wait(Duration::from_millis(1)),
        "a panicking worker must still signal completion to its closers",
    );
}
