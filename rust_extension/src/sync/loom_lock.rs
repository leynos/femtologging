//! Loom locks with `parking_lot`'s calling convention.
//!
//! `parking_lot` returns a guard directly where Loom, following the standard
//! library, returns a `LockResult`. These wrappers unwrap it, recovering the
//! guard from a poisoned lock as `parking_lot` (which never poisons) would, so
//! a caller cannot tell which arm of the seam it was compiled against.

use std::sync::PoisonError;

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
    /// Create a read-write lock holding `value`.
    pub(crate) fn new(value: T) -> Self {
        Self(loom::sync::RwLock::new(value))
    }

    /// Acquire shared access, waiting while a writer holds the lock.
    pub(crate) fn read(&self) -> loom::sync::RwLockReadGuard<'_, T> {
        self.0.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Acquire exclusive access, waiting while any other thread holds it.
    pub(crate) fn write(&self) -> loom::sync::RwLockWriteGuard<'_, T> {
        self.0.write().unwrap_or_else(PoisonError::into_inner)
    }
}
