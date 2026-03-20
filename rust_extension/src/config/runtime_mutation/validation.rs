//! Validation helpers for runtime attachment mutations.

use std::collections::BTreeSet;

use crate::{config::ConfigError, manager::LoggerAttachmentState};

use super::{CollectionMutation, SharedFilters, SharedHandlers};

pub(crate) fn collection_conflict(
    kind: &str,
    current: &CollectionMutation,
    new: &CollectionMutation,
) -> Option<String> {
    match current {
        CollectionMutation::Unchanged => None,
        _ if current == new => None,
        _ => Some(format!("multiple {kind} mutation modes were requested")),
    }
}

pub(crate) fn resolve_attachment_ids(
    state: &LoggerAttachmentState,
    handlers: &SharedHandlers,
    filters: &SharedFilters,
) -> Result<(), ConfigError> {
    let missing_handlers = state
        .handler_ids()
        .iter()
        .filter(|id| !handlers.contains_key(*id))
        .cloned();
    let missing_filters = state
        .filter_ids()
        .iter()
        .filter(|id| !filters.contains_key(*id))
        .cloned();
    let missing = missing_handlers.chain(missing_filters).collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(ConfigError::UnknownIds(missing))
    }
}

pub(crate) fn validate_remove_ids(
    existing: &[String],
    mutation: &CollectionMutation,
) -> Result<(), ConfigError> {
    let CollectionMutation::Remove(ids) = mutation else {
        return Ok(());
    };
    let existing = existing.iter().collect::<BTreeSet<_>>();
    let missing = ids
        .iter()
        .filter(|id| !existing.contains(*id))
        .cloned()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(ConfigError::UnknownIds(missing))
    }
}
