//! Logger helpers for runtime mutation workflows and logger monitoring.

use crate::filters::FemtoFilter;
use pyo3::prelude::*;
use std::sync::Arc;

use super::{FemtoLogger, HandlerAttachment};

impl FemtoLogger {
    /// Replace handler attachments while retaining each configured filter chain.
    /// Records already queued keep their handler snapshots; later records use
    /// the replacement attachments.
    pub(crate) fn replace_handlers(&self, handlers: Vec<HandlerAttachment>) {
        *self.handlers.write() = handlers;
    }

    /// Replace the complete filter collection while holding its write lock.
    /// Records already past producer filtering are unaffected; later records
    /// use the replacement filters.
    pub(crate) fn replace_filters(&self, filters: Vec<Arc<dyn FemtoFilter>>) {
        *self.filters.write() = filters;
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
