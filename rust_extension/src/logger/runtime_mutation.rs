//! Logger helpers for runtime mutation workflows and logger monitoring.

use pyo3::prelude::*;
use std::sync::Arc;

use crate::{filters::FemtoFilter, handler::FemtoHandlerTrait};
use super::{FemtoLogger, HandlerAttachment};

impl FemtoLogger {
    /// Replace the complete handler collection while holding its write lock.
    /// Records queued before this call retain their captured handler snapshot;
    /// subsequent records observe the replacement.
    pub(crate) fn replace_handlers(&self, handlers: Vec<Arc<dyn FemtoHandlerTrait>>) {
        *self.handlers.write() = handlers.into_iter().map(HandlerAttachment::new).collect();
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
