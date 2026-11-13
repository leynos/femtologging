//! Synchronise on the test-only stream-handler waiter counter.
//!
//! A `flush()` or `close()` raises the counter while it is inside its wait.
//! Tests that must prove a call was still waiting use this helper to release a
//! gated worker only after the call reached the wait, so the claim does not
//! depend on elapsed time or on the closer having started before the check.

use std::time::{Duration, Instant};

use _femtologging_rs::stream_waiter_count;

/// Longest a test waits for a call to reach its wait before failing.
///
/// The counter changes as soon as the call enters its wait, so a passing test
/// observes it almost immediately; the bound only stops a broken test from
/// hanging.
const WAITER_WAIT: Duration = Duration::from_secs(10);

/// Poll interval for the counter, which changes only at wait boundaries.
const POLL_INTERVAL: Duration = Duration::from_millis(1);

/// Block until at least `expected` stream-handler calls are waiting.
///
/// Panics rather than returning early when the count never arrives, so a test
/// cannot skip the state it meant to exercise and pass vacuously.
pub fn wait_for_waiters(expected: usize) {
    let deadline = Instant::now() + WAITER_WAIT;
    while Instant::now() < deadline {
        if stream_waiter_count() >= expected {
            return;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    panic!(
        "expected {expected} stream-handler call(s) waiting within {WAITER_WAIT:?}, saw {}",
        stream_waiter_count()
    );
}
