//! Logger helpers for whole-collection runtime mutation workflows.

use std::sync::Arc;

use super::FemtoLogger;
use crate::{filters::FemtoFilter, handler::FemtoHandlerTrait};

impl FemtoLogger {
    pub(crate) fn replace_handlers(&self, handlers: Vec<Arc<dyn FemtoHandlerTrait>>) {
        *self.handlers.write() = handlers;
    }

    pub(crate) fn replace_filters(&self, filters: Vec<Arc<dyn FemtoFilter>>) {
        *self.filters.write() = filters;
    }
}
