//! `parking_lot`'s condition variable behind [`sync`]'s calling convention.
//!
//! `parking_lot` and Loom disagree about how a timed wait is written down.
//! `parking_lot::Condvar::wait_for` borrows the guard mutably and returns
//! nothing; `loom::sync::Condvar::wait_timeout` takes the guard by value and
//! hands it back with a verdict, because it has to return it through its own
//! bookkeeping. A caller that matched the phase it waited on would have to name
//! one of those shapes, which is the conditional the seam exists to keep out of
//! the callers.
//!
//! This wrapper takes the Loom shape - guard by value, nothing returned - and
//! implements it over `parking_lot` by dropping the guard at the end. Releasing
//! the lock and reacquiring it is what both arms do; the extra acquisition here
//! is the price of one calling convention rather than two.
//!
//! [`sync`]: crate::sync

use std::time::Duration;

use parking_lot::MutexGuard;

/// A `parking_lot` condition variable with Loom's timed-wait signature.
pub(crate) struct Condvar(parking_lot::Condvar);

impl Condvar {
    /// Create a condition variable that has not been notified yet.
    pub(crate) fn new() -> Self {
        Self(parking_lot::Condvar::new())
    }

    /// Release `guard` and wait for a notification, or for `timeout` to elapse.
    ///
    /// The lock is held again when this returns, and the guard that held it is
    /// dropped here rather than handed back, so a caller re-locks to read the
    /// state it waited on. Nothing is returned: every caller in this crate
    /// re-reads that state to decide what to do, which is a sharper answer than
    /// whether the wait happened to time out.
    ///
    /// A notification that arrives without the state changing is treated as a
    /// spurious wakeup, which is what the standard library's guidance asks of
    /// any caller waiting to observe a condition.
    pub(crate) fn wait_timeout<T>(&self, mut guard: MutexGuard<'_, T>, timeout: Duration) {
        self.0.wait_for(&mut guard, timeout);
    }

    /// Wake every waiting thread.
    pub(crate) fn notify_all(&self) {
        self.0.notify_all();
    }
}
