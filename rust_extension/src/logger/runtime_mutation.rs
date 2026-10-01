//! Logger helpers for whole-collection runtime mutation workflows.

use std::sync::Arc;

use crate::filters::FemtoFilter;

use super::{FemtoLogger, HandlerAttachment};

impl FemtoLogger {
    pub(crate) fn replace_handlers(&self, handlers: Vec<HandlerAttachment>) {
        *self.handlers.write() = handlers;
    }

    pub(crate) fn replace_filters(&self, filters: Vec<Arc<dyn FemtoFilter>>) {
        *self.filters.write() = filters;
    }
}
