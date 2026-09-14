//! Logger helpers for whole-collection runtime mutation workflows.

use std::sync::Arc;

use crate::{filters::FemtoFilter, handler::FemtoHandlerTrait};

use super::FemtoLogger;

impl FemtoLogger {
    /// Replace the complete handler collection while holding its write lock.
    /// Records queued before this call retain their captured handler snapshot;
    /// subsequent records observe the replacement.
    pub(crate) fn replace_handlers(&self, handlers: Vec<Arc<dyn FemtoHandlerTrait>>) {
        *self.handlers.write() = handlers;
    }

    /// Replace the complete filter collection while holding its write lock.
    /// Records already past producer filtering are unaffected; later records
    /// use the replacement filters.
    pub(crate) fn replace_filters(&self, filters: Vec<Arc<dyn FemtoFilter>>) {
        *self.filters.write() = filters;
    }
}
