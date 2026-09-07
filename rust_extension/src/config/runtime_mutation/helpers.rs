//! Helpers for applying runtime logger attachment mutations.

use std::collections::BTreeMap;

use crate::manager::LoggerAttachmentState;

use super::{
    CollectionMutation, ConfigError, LoggerMutationBuilder, MutationRegistries,
    resolve_attachment_ids, validate_remove_ids,
};

pub(super) fn apply_mutation_to_logger(
    name: &str,
    mutation: &LoggerMutationBuilder,
    logger_states: &mut BTreeMap<String, LoggerAttachmentState>,
    registries: &MutationRegistries<'_>,
) -> Result<(), ConfigError> {
    let existing = match logger_states.get(name).cloned() {
        Some(existing) => existing,
        None if requires_existing_baseline(&mutation.handlers)
            || requires_existing_baseline(&mutation.filters) =>
        {
            return Err(ConfigError::InvalidMutation(format!(
                "{name}: logger has no runtime metadata; Append/Remove require prior build_and_init()",
            )));
        }
        None => LoggerAttachmentState::default(),
    };
    validate_remove_ids(existing.handler_ids(), &mutation.handlers)?;
    validate_remove_ids(existing.filter_ids(), &mutation.filters)?;
    let next = LoggerAttachmentState::new(
        mutation.handlers.apply(existing.handler_ids()),
        mutation.filters.apply(existing.filter_ids()),
    );
    resolve_attachment_ids(&next, registries.handlers, registries.filters)?;
    logger_states.insert(name.to_owned(), next);
    Ok(())
}

const fn requires_existing_baseline(mutation: &CollectionMutation) -> bool {
    matches!(
        mutation,
        CollectionMutation::Append(ids) | CollectionMutation::Remove(ids) if !ids.is_empty()
    )
}
