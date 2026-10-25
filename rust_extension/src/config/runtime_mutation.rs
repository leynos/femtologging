//! Runtime mutation builders and orchestration for live logger reconfiguration.

use std::{collections::BTreeMap, sync::Arc};

use pyo3::prelude::*;

use crate::{
    FemtoLevel,
    filters::{FemtoFilter, FilterBuilder},
    handler::FemtoHandlerTrait,
    manager::LoggerAttachmentState,
};

use super::{ConfigError, types::HandlerBuilder};

mod apply;
mod collection_mutation;
mod commit;
mod logger_mutation;
mod validation;

pub(crate) use collection_mutation::CollectionMutation;
pub(crate) use commit::{
    BuiltRegistries, CommitInputs, PlannedMutation, apply_commit, build_filters, build_handlers,
};
pub use logger_mutation::LoggerMutationBuilder;
pub(crate) use validation::{
    KnownIds, collection_conflict, resolve_attachment_ids, validate_remove_ids,
};

/// Builder for transactional runtime reconfiguration.
///
/// Applies handler and filter mutations against live manager state without
/// requiring a full `ConfigBuilder.build_and_init()` rebuild.
#[cfg_attr(feature = "python", pyclass(from_py_object))]
#[derive(Clone, Debug, Default)]
pub struct RuntimeConfigBuilder {
    /// Handler definitions to build and add to the commit registry.
    handlers: BTreeMap<String, HandlerBuilder>,
    /// Filter definitions to build and add to the commit registry.
    filters: BTreeMap<String, FilterBuilder>,
    /// Named logger mutations applied during the commit.
    loggers: BTreeMap<String, LoggerMutationBuilder>,
    /// Optional mutation for the root logger; mutually exclusive with a named `root` entry.
    root_logger: Option<LoggerMutationBuilder>,
}

/// Shared handler registry carried between runtime snapshots and commits.
pub(crate) type SharedHandlers = BTreeMap<String, Arc<dyn FemtoHandlerTrait>>;
/// Shared filter registry carried between runtime snapshots and commits.
pub(crate) type SharedFilters = BTreeMap<String, Arc<dyn FemtoFilter>>;

/// Scalar logger changes separated from attachment collection changes for commit application.
pub(crate) struct LoggerScalarMutation {
    /// Replacement level, when the request supplied one.
    pub(crate) level: Option<FemtoLevel>,
    /// Replacement propagation flag, when the request supplied one.
    pub(crate) propagate: Option<bool>,
}

/// Applies one logger's attachment mutation, validating its baseline and registry IDs.
fn apply_mutation_to_logger(
    name: &str,
    mutation: &LoggerMutationBuilder,
    logger_states: &mut BTreeMap<String, LoggerAttachmentState>,
    known: &KnownIds,
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
    resolve_attachment_ids(&next, known)?;
    logger_states.insert(name.to_string(), next);
    Ok(())
}
/// Reports whether a non-empty append or remove operation needs existing metadata.
fn requires_existing_baseline(mutation: &CollectionMutation) -> bool {
    matches!(
        mutation,
        CollectionMutation::Append(ids) | CollectionMutation::Remove(ids) if !ids.is_empty()
    )
}
#[cfg(feature = "python")]
mod python_bindings;
