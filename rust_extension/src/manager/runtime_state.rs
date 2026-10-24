//! Python handler, filter and attachment state stored beside the logger map.
//!
//! Keeping this snapshot separate lets the registry focus on logger identity
//! and cardinality while configuration code can stage complete runtime state.

use std::{collections::BTreeMap, sync::Arc};

use crate::{filters::FemtoFilter, handler::FemtoHandlerTrait};

/// Handler and filter identifiers attached to one registered logger.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LoggerAttachmentState {
    /// Handler identifiers retained for this logger.
    handler_ids: Vec<String>,
    /// Filter identifiers retained for this logger.
    filter_ids: Vec<String>,
}

impl LoggerAttachmentState {
    /// Record the handler and filter identifiers attached to a logger.
    pub(crate) fn new(handler_ids: Vec<String>, filter_ids: Vec<String>) -> Self {
        Self {
            handler_ids,
            filter_ids,
        }
    }

    /// Return the identifiers for handlers attached to the logger.
    pub(crate) fn handler_ids(&self) -> &[String] {
        &self.handler_ids
    }

    /// Return the identifiers for filters attached to the logger.
    pub(crate) fn filter_ids(&self) -> &[String] {
        &self.filter_ids
    }
}

/// Handler instances retained by the Python runtime configuration.
pub(crate) type SharedHandlers = BTreeMap<String, Arc<dyn FemtoHandlerTrait>>;
/// Filter instances retained by the Python runtime configuration.
pub(crate) type SharedFilters = BTreeMap<String, Arc<dyn FemtoFilter>>;

/// Runtime configuration committed alongside the logger registry.
#[derive(Clone, Default)]
pub(crate) struct RuntimeStateSnapshot {
    /// Handler instances addressable by their configuration identifiers.
    pub(crate) handler_registry: SharedHandlers,
    /// Filter instances addressable by their configuration identifiers.
    pub(crate) filter_registry: SharedFilters,
    /// Attachment identifiers for each logger name.
    pub(crate) logger_states: BTreeMap<String, LoggerAttachmentState>,
}
