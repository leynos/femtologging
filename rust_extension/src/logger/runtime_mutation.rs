//! Logger helpers for whole-collection runtime mutation workflows.

use std::sync::Arc;

use crate::{filters::FemtoFilter, handler::FemtoHandlerTrait, level::FemtoLevel};

use super::FemtoLogger;

#[derive(Clone)]
pub(crate) struct LoggerRuntimeSnapshot {
    pub(crate) level: FemtoLevel,
    pub(crate) propagate: bool,
    pub(crate) handlers: Vec<Arc<dyn FemtoHandlerTrait>>,
    pub(crate) filters: Vec<Arc<dyn FemtoFilter>>,
}

impl FemtoLogger {
    pub(crate) fn snapshot_runtime_state(&self) -> LoggerRuntimeSnapshot {
        LoggerRuntimeSnapshot {
            level: self.get_level(),
            propagate: self.propagate(),
            handlers: self.handlers.read().clone(),
            filters: self.filters.read().clone(),
        }
    }

    pub(crate) fn restore_runtime_state(&self, snapshot: &LoggerRuntimeSnapshot) {
        self.set_level(snapshot.level);
        self.set_propagate(snapshot.propagate);
        self.replace_handlers(snapshot.handlers.clone());
        self.replace_filters(snapshot.filters.clone());
    }

    pub(crate) fn replace_handlers(&self, handlers: Vec<Arc<dyn FemtoHandlerTrait>>) {
        *self.handlers.write() = handlers;
    }

    pub(crate) fn replace_filters(&self, filters: Vec<Arc<dyn FemtoFilter>>) {
        *self.filters.write() = filters;
    }
}
