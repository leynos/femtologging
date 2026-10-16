//! Loom locks with `parking_lot`'s calling convention.
//!
//! `parking_lot` returns a guard directly where Loom, following the standard
//! library, returns a `LockResult`. These wrappers unwrap it, recovering the
//! guard from a poisoned lock as `parking_lot` (which never poisons) would, so
//! a caller cannot tell which arm of the seam it was compiled against.

use std::sync::PoisonError;
use std::time::Duration;

/// A Loom mutex whose `lock` returns the guard directly.
pub(crate) struct Mutex<T>(loom::sync::Mutex<T>);

impl<T> Mutex<T> {
    /// Create a mutex holding `value`.
    pub(crate) fn new(value: T) -> Self {
        Self(loom::sync::Mutex::new(value))
    }

    /// Acquire the lock, waiting while another thread holds it.
    pub(crate) fn lock(&self) -> loom::sync::MutexGuard<'_, T> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A Loom read-write lock whose `read` and `write` return guards directly.
pub(crate) struct RwLock<T>(loom::sync::RwLock<T>);

impl<T> RwLock<T> {
    /// Acquire shared access, waiting while a writer holds it.
    pub(crate) fn read(&self) -> loom::sync::RwLockReadGuard<'_, T> {
        self.0.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Acquire exclusive access, waiting while any other thread holds it.
    pub(crate) fn write(&self) -> loom::sync::RwLockWriteGuard<'_, T> {
        self.0.write().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A Loom condition variable with Loom's own timed-wait signature.
///
/// The sibling wrapper over `parking_lot` in `sync/parking_condvar.rs` takes
/// the guard by value and returns nothing, and so does this one. Loom hands the
/// guard back alongside a verdict; the guard is dropped here, and the verdict
/// is discarded because no caller in this crate reads it - they re-read the
/// state they waited on instead.
pub(crate) struct Condvar(loom::sync::Condvar);

impl Condvar {
    /// Create a condition variable that has not been notified yet.
    pub(crate) fn new() -> Self {
        Self(loom::sync::Condvar::new())
    }

    /// Release `guard` and wait for a notification, or for `timeout` to
    /// elapse, then reacquire the lock.
    ///
    /// Loom measures the timeout in its own virtual time, so a model that waits
    /// advances the schedule rather than the clock.
    pub(crate) fn wait_timeout<T>(&self, guard: loom::sync::MutexGuard<'_, T>, timeout: Duration) {
        let (guard, _) = self
            .0
            .wait_timeout(guard, timeout)
            .unwrap_or_else(PoisonError::into_inner);
        // The guard Loom handed back is dropped here, inside the call, so the
        // lock is released exactly as the `parking_lot` arm's does: neither
        // leaves it held for the caller to trip over.
        drop(guard);
    }

    /// Wake every waiting thread.
    pub(crate) fn notify_all(&self) {
        self.0.notify_all();
    }
}
