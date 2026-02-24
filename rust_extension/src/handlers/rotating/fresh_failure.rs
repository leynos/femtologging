//! Shared state for forcing rotating writer reopen failures during tests.

use once_cell::sync::Lazy;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
#[cfg(test)]
use std::thread::{self, ThreadId};

struct FreshFailureState {
    remaining: AtomicUsize,
    reason: Mutex<Option<String>>,
    #[cfg(test)]
    owner: Mutex<Option<ThreadId>>,
}

impl FreshFailureState {
    const fn new() -> Self {
        Self {
            remaining: AtomicUsize::new(0),
            reason: Mutex::new(None),
            #[cfg(test)]
            owner: Mutex::new(None),
        }
    }

    #[cfg(feature = "python")]
    fn set_forced(&self, count: usize, reason: String) {
        self.remaining.store(count, Ordering::SeqCst);
        *self
            .reason
            .lock()
            .expect("fresh failure reason mutex poisoned") = Some(reason);
    }

    #[cfg(feature = "python")]
    fn clear_forced(&self) {
        self.remaining.store(0, Ordering::SeqCst);
        *self
            .reason
            .lock()
            .expect("fresh failure reason mutex poisoned") = None;
    }

    /// Attempts to consume one forced fresh-file-open failure.
    ///
    /// Atomically decrements the remaining failure count and returns the
    /// associated failure reason. When the count reaches zero, the reason is
    /// cleared.
    ///
    /// # Concurrency
    ///
    /// This method is safe to call concurrently with other `take()` calls.
    /// Calling the setup or teardown helpers while a take is in flight may yield
    /// inconsistent state (for example the counter being updated while the
    /// stored reason is cleared). Test code must serialize setup and teardown
    /// with respect to exercising the handler.
    fn take(&self) -> Option<String> {
        #[cfg(test)]
        {
            let current_thread = thread::current().id();
            let owner = *self
                .owner
                .lock()
                .expect("fresh failure owner mutex poisoned");
            if let Some(owner_thread) = owner {
                if owner_thread != current_thread {
                    return None;
                }
            }
        }
        let previous = self
            .remaining
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                if current == 0 {
                    None
                } else {
                    Some(current - 1)
                }
            })
            .ok()?;

        let mut guard = self
            .reason
            .lock()
            .expect("fresh failure reason mutex poisoned");
        let reason = guard.clone();
        if previous == 1 {
            *guard = None;
        }
        reason
    }
}

static FRESH_FAILURE_STATE: Lazy<FreshFailureState> = Lazy::new(FreshFailureState::new);

/// Attempts to consume one forced fresh-file-open failure.
///
/// Atomically decrements the remaining failure count and returns the associated
/// failure reason. When the count reaches zero, the reason is cleared.
///
/// # Returns
///
/// `Some(reason)` if failures remain, `None` otherwise.
pub(crate) fn take_forced_fresh_failure_reason() -> Option<String> {
    FRESH_FAILURE_STATE.take()
}

/// Configures forced fresh-file-open failures for testing.
///
/// Sets the number of times [`take_forced_fresh_failure_reason`] will return a
/// failure reason before clearing it. Intended for test setup; do not call
/// concurrently with code that is exercising the handler.
///
/// # Arguments
///
/// * `count` - Number of failures to force.
/// * `reason` - Failure message to surface.
#[cfg(feature = "python")]
pub(crate) fn set_forced_fresh_failure(count: usize, reason: impl Into<String>) {
    FRESH_FAILURE_STATE.set_forced(count, reason.into());
}

/// Clears forced fresh-file-open failures.
///
/// Resets the failure count to zero and clears the stored reason. Intended for
/// test cleanup.
#[cfg(feature = "python")]
pub(crate) fn clear_forced_fresh_failure() {
    FRESH_FAILURE_STATE.clear_forced();
}

/// Forces a single fresh-file-open failure for testing.
///
/// Creates a guard that configures the global state to return a failure reason
/// once. When the guard is dropped, the previous state is restored.
///
/// # Arguments
///
/// * `reason` - Failure message to surface during the forced failure.
///
/// # Returns
///
/// A guard that restores the previous state on drop.
#[cfg(test)]
pub(crate) fn force_fresh_failure_once_for_test(
    reason: impl Into<String>,
) -> ForcedFreshFailureGuard {
    ForcedFreshFailureGuard::new(1, reason.into())
}

/// Guard that restores the forced fresh failure state when dropped.
///
/// Created by [`force_fresh_failure_once_for_test`] to provide scoped failure
/// injection during tests. Captures the existing counter and reason so they can
/// be reinstated after the forced failure completes.
#[cfg(test)]
pub(crate) struct ForcedFreshFailureGuard {
    previous_count: usize,
    previous_reason: Option<String>,
    previous_owner: Option<ThreadId>,
}

#[cfg(test)]
impl ForcedFreshFailureGuard {
    /// Creates a guard that configures the forced failure state for tests.
    ///
    /// Saves the current forced failure count and reason before installing the
    /// supplied values. The previous state is restored when the guard is
    /// dropped.
    ///
    /// # Arguments
    ///
    /// * `count` - Number of failures to force.
    /// * `reason` - Failure message to report.
    fn new(count: usize, reason: String) -> Self {
        let previous_count = FRESH_FAILURE_STATE.remaining.swap(count, Ordering::SeqCst);
        let mut guard = FRESH_FAILURE_STATE
            .reason
            .lock()
            .expect("fresh failure reason mutex poisoned");
        let previous_reason = guard.replace(reason);
        let previous_owner = {
            let mut owner_guard = FRESH_FAILURE_STATE
                .owner
                .lock()
                .expect("fresh failure owner mutex poisoned");
            owner_guard.replace(thread::current().id())
        };
        Self {
            previous_count,
            previous_reason,
            previous_owner,
        }
    }
}

#[cfg(test)]
impl Drop for ForcedFreshFailureGuard {
    /// Restores the forced failure state captured when the guard was created.
    fn drop(&mut self) {
        FRESH_FAILURE_STATE
            .remaining
            .store(self.previous_count, Ordering::SeqCst);
        let mut guard = FRESH_FAILURE_STATE
            .reason
            .lock()
            .expect("fresh failure reason mutex poisoned");
        *guard = self.previous_reason.take();
        {
            let mut owner_guard = FRESH_FAILURE_STATE
                .owner
                .lock()
                .expect("fresh failure owner mutex poisoned");
            *owner_guard = self.previous_owner;
        }
    }
}
