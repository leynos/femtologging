//! Logger helpers for whole-collection runtime mutation workflows.

use std::sync::Arc;

use crate::{filters::FemtoFilter, handler::FemtoHandlerTrait};

use super::{FemtoLogger, HandlerAttachment};

impl FemtoLogger {
    pub(crate) fn replace_handlers(&self, handlers: Vec<Arc<dyn FemtoHandlerTrait>>) {
        *self.handlers.write() = handlers.into_iter().map(HandlerAttachment::new).collect();
    }

    pub(crate) fn replace_filters(&self, filters: Vec<Arc<dyn FemtoFilter>>) {
        *self.filters.write() = filters;
    }
}
