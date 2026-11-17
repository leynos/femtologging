//! Test-only record of Python threads running during a detached wait.
//!
//! The GIL regression tests for `flush_handlers()`, `flush()` and `close()`
//! have to show that another Python thread ran *while* the call was waiting.
//! Sampling that from Python cannot: anything the extension signals at the
//! detach stays buffered until the observing thread is scheduled, so a
//! baseline taken after consuming it can be taken after the call has returned
//! and reacquired the GIL, and ticks counted in that handover would be blamed
//! on the wait.
//!
//! The observation is recorded here instead, on the Rust side. A wait marks
//! itself in flight for exactly the span of `Python::detach` (see
//! [`crate::py_detach`]), and a test thread records each moment it got the
//! interpreter by calling [`note_python_thread_ran_for_test`]. That call needs
//! the GIL, and the in-flight span is entered and cleared while the waiting
//! thread holds the GIL, so the two cannot overlap: an observation counts only
//! if it happened between the detach and the reattach. Nothing that runs before
//! the wait or after it returns can be counted, so no scheduling delay can
//! manufacture progress -- and a wait that kept the GIL counts nothing at all,
//! because no other Python thread could run during it.
//!
//! The gate is process-wide because the waits are: each Python test that uses
//! it runs in a fresh interpreter with a single handler.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use pyo3::prelude::*;

/// How long the poll between in-flight checks sleeps while detached.
const IN_FLIGHT_POLL: Duration = Duration::from_millis(1);

/// Process-wide record of waits that have detached the interpreter.
#[derive(Default)]
struct GilReleaseGate {
    /// How many waits are between `WaitInFlight::enter` and its drop.
    waits_in_flight: AtomicUsize,
    /// How many moments a Python thread observed while a wait was in flight.
    observations: AtomicUsize,
}

impl GilReleaseGate {
    /// Record that one wait has detached, or is about to.
    fn enter_wait(&self) {
        self.waits_in_flight.fetch_add(1, Ordering::SeqCst);
    }

    /// Record that a detached wait has finished.
    fn leave_wait(&self) {
        self.waits_in_flight.fetch_sub(1, Ordering::SeqCst);
    }

    /// Report whether any wait is currently detached.
    fn has_wait_in_flight(&self) -> bool {
        self.waits_in_flight.load(Ordering::SeqCst) > 0
    }

    /// Count one moment at which another Python thread held the interpreter.
    ///
    /// Returns whether the observation counted, which is `false` when no wait
    /// is detached. Counting is deliberately conditional rather than
    /// post-filtered: a sample recorded outside the wait cannot be told apart
    /// from one recorded inside it afterwards, so it is never recorded.
    fn note_observation(&self) -> bool {
        if !self.has_wait_in_flight() {
            return false;
        }
        self.observations.fetch_add(1, Ordering::SeqCst);
        true
    }

    /// Read how many observations have been counted since the last reset.
    fn count_observations(&self) -> usize {
        self.observations.load(Ordering::SeqCst)
    }

    /// Forget observations recorded before a test armed its measurement.
    fn reset_observations(&self) {
        self.observations.store(0, Ordering::SeqCst);
    }
}

/// The gate every detached wait reports to.
fn process_gate() -> &'static GilReleaseGate {
    static GATE: OnceLock<GilReleaseGate> = OnceLock::new();
    GATE.get_or_init(GilReleaseGate::default)
}

/// Marks one detached wait as in flight until it is dropped.
///
/// The guard lives in [`crate::py_detach::detach_for_wait`], where it is
/// dropped after `Python::detach` returns -- that is, while the waiting thread
/// holds the GIL again and before any Python code can run. A test thread
/// therefore cannot observe "in flight" after the call returned.
pub(crate) struct WaitInFlight(());

impl WaitInFlight {
    /// Mark a wait as in flight.
    pub(crate) fn enter() -> Self {
        process_gate().enter_wait();
        Self(())
    }
}

impl Drop for WaitInFlight {
    fn drop(&mut self) {
        process_gate().leave_wait();
    }
}

/// Count that this Python thread ran while a wait was detached.
///
/// Returns whether the observation counted, which is `false` when no wait is
/// detached and the call is therefore only ordinary interpreter time.
#[pyfunction]
pub(crate) fn note_python_thread_ran_for_test() -> bool {
    process_gate().note_observation()
}

/// Read how many observations were counted since the last reset.
#[pyfunction]
pub(crate) fn gil_release_observations_for_test() -> usize {
    process_gate().count_observations()
}

/// Forget observations from earlier calls before a test arms its measurement.
#[pyfunction]
pub(crate) fn reset_gil_release_observations_for_test() {
    process_gate().reset_observations();
}

/// Wait until a Python-facing wait has detached, for at most *timeout_ms*.
///
/// The wait itself releases the interpreter while it polls, so a test can use
/// it without stopping the call under test from detaching.
#[pyfunction]
pub(crate) fn wait_for_wait_in_flight_for_test(py: Python<'_>, timeout_ms: u64) -> bool {
    py.detach(|| {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        while !process_gate().has_wait_in_flight() {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(IN_FLIGHT_POLL);
        }
        true
    })
}

#[cfg(test)]
mod tests {
    //! Checks that observations count only while a wait is in flight.

    use super::GilReleaseGate;

    #[test]
    fn an_observation_outside_a_wait_is_not_counted() {
        let gate = GilReleaseGate::default();

        assert!(!gate.note_observation());
        assert_eq!(gate.count_observations(), 0);
    }

    #[test]
    fn an_observation_during_a_wait_is_counted() {
        let gate = GilReleaseGate::default();
        gate.enter_wait();

        assert!(gate.note_observation());

        gate.leave_wait();
        assert_eq!(gate.count_observations(), 1);
    }

    #[test]
    fn resetting_forgets_earlier_observations() {
        let gate = GilReleaseGate::default();
        gate.enter_wait();
        assert!(gate.note_observation());
        gate.leave_wait();

        gate.reset_observations();

        assert_eq!(gate.count_observations(), 0);
    }
}
