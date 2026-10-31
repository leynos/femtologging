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
    /// Detach every handler and filter, returning the displaced owners.
    ///
    /// Companion to [`Self::replace_handlers`] and [`Self::replace_filters`]
    /// for callers that clear rather than replace, so a transaction can defer
    /// the same releases past its guard.
    pub(crate) fn detach_owners(&self) -> DetachedOwners {
        DetachedOwners {
            attachments: std::mem::take(&mut *self.handlers.write()),
            filters: std::mem::take(&mut *self.filters.write()),
        }
    }

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
        std::mem::replace(&mut *self.handlers.write(), handlers)
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
        std::mem::replace(&mut *self.filters.write(), filters)
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
