//! Logger helpers for runtime mutation workflows and logger monitoring.

use crate::filters::FemtoFilter;
use pyo3::prelude::*;
use std::sync::Arc;

use super::{FemtoLogger, HandlerAttachment};

/// Owners detached from one logger, retained so a caller can defer their drop.
///
/// Holding the collections is the entire purpose of this type: dropping an
/// attachment shuts down its worker and dropping a filter may release the last
/// Python reference for a callback. Callers keep this bundle alive until the
/// configuration transaction that displaced it has released its guard, then
/// call [`Self::release`].
#[cfg(feature = "python")]
pub(crate) struct DetachedOwners {
    /// Handler attachments removed from the logger.
    pub(crate) attachments: Vec<HandlerAttachment>,
    /// Filters removed from the logger.
    pub(crate) filters: Vec<Arc<dyn FemtoFilter>>,
}

#[cfg(feature = "python")]
impl DetachedOwners {
    /// Drop the held handlers and filters.
    pub(crate) fn release(self) {
        drop(self.attachments);
        drop(self.filters);
    }
}

impl FemtoLogger {
    /// Replace handler attachments while retaining each configured filter chain.
    ///
    /// Returns the displaced attachments rather than dropping them, so a caller
    /// holding a transaction guard can release the last handler owner only after
    /// that guard is gone. Records already queued keep their handler snapshots;
    /// later records use the replacement attachments.
    pub(crate) fn replace_handlers(
        &self,
        handlers: Vec<HandlerAttachment>,
    ) -> Vec<HandlerAttachment> {
        // Destructors and finalizers must not run while a collection guard is
        // held, so the displaced attachments leave the guarded block intact and
        // the caller drops them once its own guard and the GIL are released.
        let mut current = self.handlers.write();
        let displaced = std::mem::replace(&mut *current, handlers);
        drop(current);
        displaced
    }

    /// Replace the complete filter collection while holding its write lock.
    ///
    /// Returns the displaced filters for the same reason as
    /// [`Self::replace_handlers`]: a Python callback filter may own the last
    /// reference to a callable whose finalizer re-enters configuration.
    /// Records already past producer filtering are unaffected; later records
    /// use the replacement filters.
    pub(crate) fn replace_filters(
        &self,
        filters: Vec<Arc<dyn FemtoFilter>>,
    ) -> Vec<Arc<dyn FemtoFilter>> {
        // As with handlers: the old filters outlive the guard and are dropped
        // by the caller.
        let mut current = self.filters.write();
        let displaced = std::mem::replace(&mut *current, filters);
        drop(current);
        displaced
    }
}

#[pymethods]
impl FemtoLogger {
    /// Return the number of records dropped because Rust context validation failed.
    ///
    /// This counter is separate from [`Self::get_dropped`], which reports
    /// records discarded because the logger queue was full.
    #[pyo3(text_signature = "(self)")]
    pub fn get_context_dropped(&self) -> u64 {
        self.context_dropped_records
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}
