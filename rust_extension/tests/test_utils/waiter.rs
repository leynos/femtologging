//! Serialise and observe the test-only stream-handler waiter counter.
//!
//! A `flush()` or `close()` raises the counter for the whole time it is inside
//! a blocking call: it is raised immediately before the call that may block and
//! lowered when that call returns. Tests that must prove a call was waiting use
//! this helper to release a gated worker only after their own call reached that
//! state, so the claim does not depend on elapsed time or on the caller having
//! started before the check.
//!
//! The counter is process-wide, so every `flush()` or `close()` in the same
//! test binary moves it, including one that returns at once and one raised by
//! dropping a handler. Each test in such a binary therefore holds
//! [`serialise`] for its duration; without that, a test observing the counter
//! could see another test's call and proceed before its own reached the wait.
//! The observation additionally measures a rise above the count it saw first,
//! so a call left waiting by an earlier test cannot satisfy the handshake.

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

/// Serialises every test in one binary that can move the process-wide counter.
static WAIT_COUNTER_LOCK: Mutex<()> = Mutex::new(());

/// Exclusive hold on the waiter counter for one test's duration.
///
/// Hold it for the whole test, not only around the call under test: a handler
/// dropped during teardown closes itself, and that close moves the same
/// counter.
pub struct WaiterSerial {
    /// Held until the test that took it finishes.
    _serial: MutexGuard<'static, ()>,
}

/// Take the waiter counter exclusively.
///
/// Every test in a binary that uses this module holds the guard, so the only
/// calls that can move the counter while a test observes it belong to that
/// test.
pub fn serialise() -> WaiterSerial {
    WaiterSerial {
        _serial: WAIT_COUNTER_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    }
}

/// Exclusive view of the waiter counter, measured from a recorded baseline.
pub struct WaiterObservation {
    /// Held for as long as this test observes the counter.
    _serial: WaiterSerial,
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
        let serial = serialise();
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
