//! Configuration builders for femtologging.

#[cfg(feature = "python")]
use once_cell::sync::Lazy;
#[cfg(feature = "python")]
use parking_lot::{Mutex, MutexGuard};
#[cfg(feature = "python")]
use pyo3::Python;

#[cfg(feature = "python")]
use crate::{logger::DetachedOwners, manager::RuntimeStateSnapshot};

#[cfg(feature = "python")]
mod build;
mod formatter_builder;
mod handler_builder;
#[cfg(feature = "python")]
mod py;
#[cfg(feature = "python")]
mod reachability;
#[cfg(feature = "python")]
mod runtime_mutation;
mod types;

/// Serializes runtime planning and publication across configuration paths.
#[cfg(feature = "python")]
static RUNTIME_TRANSACTION: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// Try to take the runtime transaction guard without blocking.
///
/// Returns `None` when the guard is held, which is what the drop probes in
/// [`runtime_transaction_tests`] assert against: a displaced owner released
/// while this returns `None` on the releasing thread would have run its
/// finalizer under the transaction. `parking_lot` mutexes are not reentrant, so
/// a caller that already holds the guard also sees `None` rather than
/// deadlocking.
#[cfg(feature = "python")]
pub(crate) fn try_lock_runtime_transaction() -> Option<MutexGuard<'static, ()>> {
    RUNTIME_TRANSACTION.try_lock()
}

/// Take the runtime transaction guard, releasing the GIL while backing off.
///
/// The guard must stay in scope for the whole transaction. Polling with
/// [`Mutex::try_lock`] rather than blocking in [`Mutex::lock`] matters because
/// every caller holds the GIL: a blocking acquisition could park a GIL-holding
/// thread behind a transaction that needs the GIL to finish.
#[cfg(feature = "python")]
pub(crate) fn lock_runtime_transaction(py: Python<'_>) -> MutexGuard<'static, ()> {
    loop {
        if let Some(transaction) = try_lock_runtime_transaction() {
            return transaction;
        }
        py.detach(std::thread::yield_now);
    }
}

/// Owners displaced by a commit, held until the transaction guard is released.
///
/// Dropping a handler attachment shuts down its worker and dropping a filter can
/// release the last `Py<PyAny>` for a Python callback, which runs a finalizer.
/// A finalizer may re-enter configuration and try to take the same transaction
/// guard, so these owners must outlive it: this mutex is not reentrant, and the
/// re-entrant call would deadlock the thread that already holds it.
#[cfg(feature = "python")]
#[derive(Default)]
pub(crate) struct DeferredDrop {
    /// Handlers and filters detached from live loggers.
    pub(crate) detached: Vec<DetachedOwners>,
    /// Runtime registry snapshots replaced by publication.
    pub(crate) snapshots: Vec<RuntimeStateSnapshot>,
}

#[cfg(feature = "python")]
impl DeferredDrop {
    /// Release every deferred owner.
    ///
    /// Call this only after the runtime transaction guard is gone. Naming the
    /// release keeps the ordering explicit at each call site instead of relying
    /// on a bare `drop`, and gives the one place that explains why these owners
    /// outlive the transaction.
    pub(crate) fn release(self) {
        for detached in self.detached {
            detached.release();
        }
        drop(self.snapshots);
    }
}
// Re-export for external consumers.
pub use formatter_builder::FormatterBuilder;
#[cfg_attr(
    not(feature = "python"),
    expect(unused_imports, reason = "public re-exports for Python-enabled builds")
)]
#[cfg(feature = "python")]
pub use runtime_mutation::{LoggerMutationBuilder, RuntimeConfigBuilder};
pub(crate) use types::normalize_vec;
#[cfg_attr(
    not(feature = "python"),
    expect(unused_imports, reason = "public re-exports for external consumers")
)]
pub use types::{ConfigBuilder, ConfigError, HandlerBuilder, LoggerConfigBuilder};

#[cfg(all(test, feature = "python"))]
mod build_tests;
#[cfg(all(test, feature = "python"))]
mod config_tests;
#[cfg(all(test, feature = "python"))]
mod propagate_tests;
#[cfg(all(test, feature = "python"))]
mod reachability_tests;
#[cfg(all(test, feature = "python"))]
mod runtime_lifecycle_tests;
#[cfg(all(test, feature = "python"))]
mod runtime_mutation_tests;
#[cfg(all(test, feature = "python"))]
mod runtime_pruning_tests;
#[cfg(all(test, feature = "python"))]
mod runtime_transaction_tests;
#[cfg(all(test, feature = "python"))]
mod test_utils;
