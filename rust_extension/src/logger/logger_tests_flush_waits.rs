//! Bounded waits the flush tests use to observe a shard's queue state.
//!
//! A flush test has to know that another thread reached a particular point —
//! its marker queued, its caller blocked for room — before it changes the
//! state that point depends on. Sleeping and hoping passes just as well when
//! the thread never got there, so each wait polls a figure the shard or the
//! caller actually publishes and fails the test if the deadline passes. They
//! live here so the two worker-level test modules share one definition.

use std::thread::ScopedJoinHandle;
use std::time::{Duration, Instant};

use super::Shard;
use super::logger_tests_flush::flush_bound;

/// Block until the shard reports `expected` queued jobs.
///
/// Yields politely rather than sleeping so the shard worker can drain, and
/// fails the test if the queue never reaches that depth within the flush bound.
pub(super) fn wait_for_queued_jobs(shard: &Shard, expected: usize) {
    let deadline = Instant::now() + flush_bound();
    while shard.queued_jobs_for_test() != expected && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(
        shard.queued_jobs_for_test(),
        expected,
        "the shard queue should hold {expected} job(s) while the worker is blocked"
    );
}

/// Wait until the shard counts `expected` flush callers blocked in admission.
///
/// A spawned caller has not necessarily reached the queue yet, so waiting for
/// this count is what turns "a marker is still waiting for room" into an
/// observation. A sleep would pass just as well for a caller that never got
/// that far.
pub(super) fn wait_for_marker_waiters(shard: &Shard, expected: usize) {
    let deadline = Instant::now() + flush_bound();
    while shard.marker_waiters_for_test() != expected && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(
        shard.marker_waiters_for_test(),
        expected,
        "the shard should have {expected} flush caller(s) waiting for queue room"
    );
}

/// Report whether every spawned flush caller finished within the flush bound.
pub(super) fn all_flush_callers_finished(callers: &[&ScopedJoinHandle<'_, bool>]) -> bool {
    let deadline = Instant::now() + flush_bound();
    let have_finished = || callers.iter().all(|caller| caller.is_finished());
    while !have_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    have_finished()
}
