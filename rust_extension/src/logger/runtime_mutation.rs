//! Logger helpers for whole-collection runtime mutation workflows.

use std::sync::Arc;

use crate::{filters::FemtoFilter, handler::FemtoHandlerTrait};

use super::FemtoLogger;

impl FemtoLogger {
    /// Maintains logger lifecycle and propagation semantics across Python calls and the background delivery runtime.
    pub(crate) fn replace_handlers(&self, handlers: Vec<Arc<dyn FemtoHandlerTrait>>) {
        *self.handlers.write() = handlers;
    }

    /// Maintains logger lifecycle and propagation semantics across Python calls and the background delivery runtime.
    pub(crate) fn replace_filters(&self, filters: Vec<Arc<dyn FemtoFilter>>) {
        *self.filters.write() = filters;
    }
}
