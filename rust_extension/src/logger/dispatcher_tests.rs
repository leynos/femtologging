//! Tests for dispatch-pool sizing and fallible initialization.
//!
//! Logger behaviour is asserted through the logger lifecycle and shutdown
//! tests. This module covers the machine-derived shard bound and injects a
//! worker-start failure to verify error mapping, reservation release and retry
//! without exhausting the process's real thread resources.

use super::super::pid_reservation::PidReservation;
use super::shard::Shard;
use super::{Dispatcher, DispatcherError, shards_for};

/// The default the pool falls back to when the machine reports no limit.
const DEFAULT: usize = super::DEFAULT_SHARD_COUNT;

/// Start a shard, refusing the second one.
///
/// The injected refusal stands in for an operating system that will not start
/// another thread, so a pool's partial-start error can be asserted without
/// exhausting the process's real thread resources.
fn refuse_second_shard(index: usize, depth: usize) -> Result<Shard, std::io::Error> {
    if index == 1 {
        return Err(std::io::Error::other("injected worker-start refusal"));
    }
    Shard::start(index, depth)
}

#[test]
fn a_machine_with_plenty_of_cores_gets_the_default_pool() {
    assert_eq!(
        shards_for(Some(64)),
        DEFAULT,
        "a 64-core machine must not turn into 64 logging threads"
    );
}

#[test]
fn a_small_machine_gets_one_shard_per_core() {
    assert_eq!(shards_for(Some(2)), 2, "two cores can use two shards");
    assert_eq!(shards_for(Some(1)), 1, "one core can use one shard");
    assert_eq!(
        shards_for(Some(DEFAULT)),
        DEFAULT,
        "a machine with exactly the cap's cores must get the cap, not one fewer"
    );
    assert_eq!(
        shards_for(Some(DEFAULT + 1)),
        DEFAULT,
        "one core past the cap must still clamp down to it"
    );
}

#[test]
fn a_machine_reporting_nothing_gets_a_usable_pool() {
    assert_eq!(
        shards_for(None),
        DEFAULT,
        "an unknown machine must still get a pool"
    );
    assert_eq!(
        shards_for(Some(0)),
        1,
        "a machine claiming zero cores must still get one shard, never none"
    );
}

/// One logger's budget leaves another logger a slot on any multi-slot shard.
///
/// A budget equal to the depth is no bound at all: on a machine reporting one
/// core the pool's depth is exactly `RECORDS_PER_LOGGER`, so a logger granted
/// the full figure could take every permit and every other logger on that
/// shard would be refused. That is the starvation the budget exists to
/// prevent, which is why the guarantee is asserted here as arithmetic for the
/// shallow cases a test can build rather than left to a deep pool it cannot.
///
/// A one-deep shard is exempt because the property is impossible there rather
/// than violated: it holds a single record, so whoever takes the permit takes
/// the shard. Production never builds one — a pool shard's depth is
/// `RECORDS_PER_LOGGER * shard_count` — and a budget of zero would be worse,
/// refusing every record on a shard that can still hold one.
#[test]
fn a_loggers_budget_leaves_another_logger_a_slot() {
    for depth in 1..=8 {
        let shard = Shard::start(0, depth).expect("a test shard should start");
        let limit = shard.budget().limit_for_test();
        assert!(limit >= 1, "a budget must never be zero on any shard");
        if depth > 1 {
            assert!(
                limit < depth,
                "a budget of {limit} on a {depth}-deep shard would let one \
                 logger take every permit"
            );
        }
        assert!(
            shard.wait_for_drain(),
            "the test shard's worker must stop cleanly"
        );
    }
}

/// A pool that would own no shard is refused when it is built.
///
/// The refusal is what makes `assign_shard` total: by the time a pool exists it
/// holds a shard, so round-robin assignment cannot look up an empty pool.
#[test]
fn a_pool_of_zero_shards_is_refused() {
    use std::io::ErrorKind;

    let Err(error) = Dispatcher::start_with(std::process::id(), 0, Shard::start) else {
        panic!("a pool with no shard must be refused");
    };
    assert_eq!(error.started, 0, "no worker start was attempted");
    assert_eq!(error.total, 0, "no shard was requested");
    assert_eq!(
        error.source.kind(),
        ErrorKind::InvalidInput,
        "an empty pool is an invalid request, not a thread refusal"
    );
    assert!(
        error.to_string().contains("at least one shard"),
        "the refusal must name the invariant: {error}"
    );

    let Err(seam_error) = super::shared_with_shards_for_test(0) else {
        panic!("the test seam must refuse an empty pool too");
    };
    assert_eq!(
        seam_error.source.kind(),
        ErrorKind::InvalidInput,
        "the test seam must report the same invalid request"
    );
}

#[test]
fn an_injected_pool_start_failure_releases_its_reservation_for_retry() {
    use std::io::ErrorKind;
    use std::sync::atomic::{AtomicU32, Ordering};

    use pyo3::{Python, exceptions::PyRuntimeError};

    let process_id = std::process::id();
    let unused_process_id = process_id.wrapping_add(1);
    let reservation = AtomicU32::new(0);
    let failed_start = {
        let start_guard = PidReservation::try_reserve(&reservation, process_id)
            .expect("the initial test reservation should be free");
        Dispatcher::start_reserved_pool_with(unused_process_id, start_guard, || {
            Dispatcher::start_with(unused_process_id, 2, refuse_second_shard)
        })
    };

    let Err(error) = failed_start else {
        panic!("the injected second worker start must fail");
    };
    assert_eq!(error.started, 1, "the error should report a partial start");
    assert_eq!(
        error.total, 2,
        "the error should report the requested pool size"
    );
    assert_eq!(error.source.kind(), ErrorKind::Other);
    assert_eq!(
        reservation.load(Ordering::Acquire),
        0,
        "a failed startup must release its reservation"
    );

    let python_error = pyo3::PyErr::from(error);
    Python::attach(|py| {
        assert!(
            python_error.is_instance_of::<PyRuntimeError>(py),
            "resource refusal should map to Python RuntimeError"
        );
    });

    let retry_guard = PidReservation::try_reserve(&reservation, process_id)
        .expect("the failed pool start must leave its reservation free for retry");
    drop(retry_guard);
    assert_eq!(
        reservation.load(Ordering::Acquire),
        0,
        "releasing a retry reservation must leave the word free"
    );
}

// `DispatcherError::new` and `invalid_pool` are `const`, which is what lets a
// refusal be described where it is raised. Both are crate-private, so an
// external `trybuild` fixture cannot name them.
//
// A `std::io::Error` cannot be built in a const item, so the constructors are
// reached through `const fn` wrappers that take the error as a parameter. A
// `const fn` body is checked for const-evaluability when it is defined, so the
// wrapped call must be `const` or this file stops compiling. Binding the
// function pointer instead would accept a non-`const` `fn` and pin nothing.
const fn error_with(started: usize, total: usize, source: std::io::Error) -> DispatcherError {
    DispatcherError::new(started, total, source)
}
#[cfg(not(loom))]
const fn invalid_with(source: std::io::Error) -> DispatcherError {
    DispatcherError::invalid_pool(source)
}
const _: fn(usize, usize, std::io::Error) -> DispatcherError = error_with;
#[cfg(not(loom))]
const _: fn(std::io::Error) -> DispatcherError = invalid_with;
