//! Synchronise on the test-only stream-handler waiter counter.
//!
//! A `flush()` or `close()` raises the counter only while it is blocked in a
//! wait. Tests that must prove a call was still waiting use this helper to
//! release a gated worker only after their own call reached that state, so the
//! claim does not depend on elapsed time or on the caller having started
//! before the check.
//!
//! The counter is process-wide, so the observation holds an exclusive lock for
//! the test's duration and measures a rise above the count it saw first. A
//! lingering call from another test can neither satisfy the handshake nor be
//! mistaken for the call under test.

use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use _femtologging_rs::stream_waiter_count;

/// Longest a test waits for its call to reach a wait before failing.
///
/// The counter changes as soon as the call blocks, so a passing test observes
/// it almost immediately; the bound only stops a broken test from hanging.
const WAITER_WAIT: Duration = Duration::from_secs(10);

/// Poll interval for the counter, which changes only at wait boundaries.
const POLL_INTERVAL: Duration = Duration::from_millis(1);

/// Serialises the tests in one binary that observe the process-wide counter.
static WAIT_COUNTER_LOCK: Mutex<()> = Mutex::new(());

/// Exclusive view of the waiter counter, measured from a recorded baseline.
pub struct WaiterObservation {
    /// Held for as long as this test observes the counter.
    _serial: MutexGuard<'static, ()>,
    /// Calls already waiting when this test took the lock.
    baseline: usize,
}

impl WaiterObservation {
    /// Take the counter exclusively and record how many calls already wait.
    ///
    /// Taking the lock before spawning the call under test means the baseline
    /// cannot include that call, so the wait below can only be satisfied by a
    /// call that started afterwards.
    pub fn acquire() -> Self {
        let serial = WAIT_COUNTER_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Self {
            _serial: serial,
            baseline: stream_waiter_count(),
        }
    }

    /// Block until a call started after [`Self::acquire`] is waiting.
    ///
    /// Panics rather than returning early when the count never rises, so a test
    /// cannot skip the state it meant to exercise and pass vacuously.
    pub fn wait_for_call(&self) {
        let deadline = Instant::now() + WAITER_WAIT;
        while Instant::now() < deadline {
            if stream_waiter_count() > self.baseline {
                return;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        panic!(
            "no stream-handler call started waiting within {WAITER_WAIT:?}; \
             count stayed at the baseline of {}",
            self.baseline
        );
    }
}
