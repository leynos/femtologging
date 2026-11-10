//! Test-only observation of stream-handler calls that are waiting.
//!
//! The regressions these tests guard against depend on whether a `flush()` or
//! `close()` call has reached its wait. Inferring that from elapsed time makes
//! the test racy on a loaded machine, so a test can instead poll the counter
//! this module maintains. The counter exists only with the `test-util`
//! feature; without it the guard records nothing.

#[cfg(feature = "test-util")]
use std::sync::atomic::{AtomicUsize, Ordering};

/// Number of stream-handler calls currently blocked in a wait.
#[cfg(feature = "test-util")]
static WAITING: AtomicUsize = AtomicUsize::new(0);

/// Marks one call as waiting for as long as the guard is alive.
pub(super) struct WaitGuard;

#[cfg(feature = "test-util")]
impl WaitGuard {
    /// Records that the calling thread is about to wait.
    pub(super) fn enter() -> Self {
        WAITING.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

#[cfg(not(feature = "test-util"))]
impl WaitGuard {
    /// Returns a guard that records nothing.
    pub(super) fn enter() -> Self {
        Self
    }
}

#[cfg(feature = "test-util")]
impl Drop for WaitGuard {
    fn drop(&mut self) {
        WAITING.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Returns how many stream-handler calls are waiting right now.
#[cfg(feature = "test-util")]
pub fn waiting_count() -> usize {
    WAITING.load(Ordering::SeqCst)
}
