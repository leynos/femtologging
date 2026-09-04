//! Logger helpers for runtime mutation workflows and logger monitoring.

use pyo3::prelude::*;
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
