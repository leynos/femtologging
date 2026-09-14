//! Runtime mutation builders and orchestration for live logger reconfiguration.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use pyo3::prelude::*;

use crate::{
    FemtoLevel,
    filters::{FemtoFilter, FilterBuilder},
    handler::FemtoHandlerTrait,
    manager::{LoggerAttachmentState, RuntimeStateSnapshot},
};

use super::{ConfigError, types::HandlerBuilder};

mod collection_mutation;
mod commit;
mod validation;

pub(crate) use collection_mutation::CollectionMutation;
pub(crate) use commit::{BuiltRegistries, apply_commit, build_filters, build_handlers};
pub(crate) use validation::{collection_conflict, resolve_attachment_ids, validate_remove_ids};

/// Builder for structured runtime mutation of a single logger.
///
/// The builder keeps scalar changes (`level`, `propagate`) separate from
/// collection changes so handlers and filters can be appended, replaced,
/// removed, or cleared explicitly.
#[cfg_attr(feature = "python", pyclass(from_py_object))]
#[derive(Clone, Debug, Default)]
pub struct LoggerMutationBuilder {
    /// Optional level override applied when this mutation is committed.
    level: Option<FemtoLevel>,
    /// Optional propagation override applied when this mutation is committed.
    propagate: Option<bool>,
    /// The one permitted handler collection operation, or unchanged by default.
    handlers: CollectionMutation,
    /// The one permitted filter collection operation, or unchanged by default.
    filters: CollectionMutation,
    /// First collection-mode conflict encountered while building this request.
    invalid: Option<String>,
}

impl LoggerMutationBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Converts generic ID inputs without changing their order; mutation constructors perform deduplication.
    fn normalize_ids<I, S>(ids: I) -> Vec<String>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        ids.into_iter().map(Into::into).collect()
    }

    /// Converts IDs to the requested mutation and records it in the selected collection field.
    fn apply_ids_mutation<I, S>(
        mut self,
        ids: I,
        mutation: impl FnOnce(Vec<String>) -> CollectionMutation,
        set: impl FnOnce(&mut Self, CollectionMutation),
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let ids = Self::normalize_ids(ids);
        set(&mut self, mutation(ids));
        self
    }

    /// Applies a replacement operation to one attachment collection.
    fn do_replace<I, S>(self, ids: I, setter: impl FnOnce(&mut Self, CollectionMutation)) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.apply_ids_mutation(ids, CollectionMutation::replace, setter)
    }

    /// Applies an append operation to one attachment collection.
    fn do_append<I, S>(self, ids: I, setter: impl FnOnce(&mut Self, CollectionMutation)) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.apply_ids_mutation(ids, CollectionMutation::append, setter)
    }

    /// Applies a removal operation to one attachment collection.
    fn do_remove<I, S>(self, ids: I, setter: impl FnOnce(&mut Self, CollectionMutation)) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.apply_ids_mutation(ids, CollectionMutation::remove, setter)
    }

    /// Applies the clear operation to one attachment collection.
    fn do_clear(self, setter: impl FnOnce(&mut Self, CollectionMutation)) -> Self {
        let mut this = self;
        setter(&mut this, CollectionMutation::Clear);
        this
    }

    pub fn with_level(mut self, level: FemtoLevel) -> Self {
        self.level = Some(level);
        self
    }

    pub fn with_propagate(mut self, propagate: bool) -> Self {
        self.propagate = Some(propagate);
        self
    }

    pub fn replace_handlers<I, S>(self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.do_replace(ids, Self::set_handlers)
    }

    pub fn append_handlers<I, S>(self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.do_append(ids, Self::set_handlers)
    }

    pub fn remove_handlers<I, S>(self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.do_remove(ids, Self::set_handlers)
    }

    pub fn clear_handlers(self) -> Self {
        self.do_clear(Self::set_handlers)
    }

    pub fn replace_filters<I, S>(self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.do_replace(ids, Self::set_filters)
    }

    pub fn append_filters<I, S>(self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.do_append(ids, Self::set_filters)
    }

    pub fn remove_filters<I, S>(self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.do_remove(ids, Self::set_filters)
    }

    pub fn clear_filters(self) -> Self {
        self.do_clear(Self::set_filters)
    }

    /// Records a conflicting handler mode while retaining the newest requested mode.
    fn set_handlers(&mut self, mutation: CollectionMutation) {
        if self.invalid.is_none() {
            self.invalid = collection_conflict("handlers", &self.handlers, &mutation);
        }
        self.handlers = mutation;
    }

    /// Records a conflicting filter mode while retaining the newest requested mode.
    fn set_filters(&mut self, mutation: CollectionMutation) {
        if self.invalid.is_none() {
            self.invalid = collection_conflict("filters", &self.filters, &mutation);
        }
        self.filters = mutation;
    }

    /// Converts a recorded collection conflict into a logger-qualified configuration error.
    fn ensure_valid(&self, logger_name: &str) -> Result<(), ConfigError> {
        self.invalid
            .clone()
            .map(|message| ConfigError::InvalidMutation(format!("{logger_name}: {message}")))
            .map_or(Ok(()), Err)
    }
}

/// Builder for transactional runtime reconfiguration.
///
/// `RuntimeConfigBuilder` applies handler and filter mutations against the
/// live manager state without requiring a full `ConfigBuilder.build_and_init()`
/// rebuild.
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

impl RuntimeConfigBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_handler<B>(mut self, id: impl Into<String>, builder: B) -> Self
    where
        B: Into<HandlerBuilder>,
    {
        self.handlers.insert(id.into(), builder.into());
        self
    }

    pub fn with_filter(mut self, id: impl Into<String>, builder: FilterBuilder) -> Self {
        self.filters.insert(id.into(), builder);
        self
    }

    pub fn with_logger(mut self, name: impl Into<String>, builder: LoggerMutationBuilder) -> Self {
        self.loggers.insert(name.into(), builder);
        self
    }

    pub fn with_root_logger(mut self, builder: LoggerMutationBuilder) -> Self {
        self.root_logger = Some(builder);
        self
    }

    /// Apply the runtime mutation transactionally.
    pub fn apply(&self) -> Result<(), ConfigError> {
        self.validate()?;
        let built = BuiltRegistries {
            handlers: build_handlers(&self.handlers)?,
            filters: build_filters(&self.filters)?,
        };

        Python::attach(|py| {
            let before = crate::manager::snapshot_runtime_state();
            let commit = self.prepare_commit(py, before, built)?;
            apply_commit(py, &commit)?;
            Ok(())
        })
    }

    /// Rejects duplicate root addressing and any previously recorded collection conflict.
    fn validate(&self) -> Result<(), ConfigError> {
        if self.root_logger.is_some() && self.loggers.contains_key("root") {
            return Err(ConfigError::InvalidMutation(
                "root logger cannot be mutated via both with_root_logger() and with_logger(\"root\", ...)"
                    .to_string(),
            ));
        }
        if let Some(root) = &self.root_logger {
            root.ensure_valid("root")?;
        }
        for (name, mutation) in &self.loggers {
            mutation.ensure_valid(name)?;
        }
        Ok(())
    }

    /// Finds loggers affected by new registry entries or explicit logger mutations.
    fn collect_impacted(&self, before: &RuntimeStateSnapshot) -> BTreeSet<String> {
        let overridden_handler_ids = self.handlers.keys().cloned().collect::<BTreeSet<_>>();
        let overridden_filter_ids = self.filters.keys().cloned().collect::<BTreeSet<_>>();
        let mut impacted = before
            .logger_states
            .iter()
            .filter(|(_, state)| {
                state
                    .handler_ids()
                    .iter()
                    .any(|id| overridden_handler_ids.contains(id))
                    || state
                        .filter_ids()
                        .iter()
                        .any(|id| overridden_filter_ids.contains(id))
            })
            .map(|(name, _)| name.clone())
            .collect::<BTreeSet<_>>();
        if self.root_logger.is_some() {
            impacted.insert("root".to_string());
        }
        impacted.extend(self.loggers.keys().cloned());
        impacted
    }
    /// Applies root and named collection mutations to a private state map before publication.
    fn apply_logger_mutations(
        &self,
        logger_states: &mut BTreeMap<String, LoggerAttachmentState>,
        handler_registry: &SharedHandlers,
        filter_registry: &SharedFilters,
    ) -> Result<(), ConfigError> {
        let root_iter = self.root_logger.iter().map(|m| ("root", m));
        let named_iter = self.loggers.iter().map(|(n, m)| (n.as_str(), m));
        for (name, mutation) in root_iter.chain(named_iter) {
            apply_mutation_to_logger(
                name,
                mutation,
                logger_states,
                handler_registry,
                filter_registry,
            )?;
        }
        Ok(())
    }
    /// Extracts scalar overrides for the logger names addressed by this request.
    fn build_scalar_mutations(&self) -> BTreeMap<String, LoggerScalarMutation> {
        let mut out = BTreeMap::new();
        if let Some(root) = &self.root_logger {
            out.insert(
                "root".to_string(),
                LoggerScalarMutation {
                    level: root.level,
                    propagate: root.propagate,
                },
            );
        }
        for (name, mutation) in &self.loggers {
            out.insert(
                name.clone(),
                LoggerScalarMutation {
                    level: mutation.level,
                    propagate: mutation.propagate,
                },
            );
        }
        out
    }
}
/// Applies one logger's attachment mutation, validating its baseline and registry IDs.
fn apply_mutation_to_logger(
    name: &str,
    mutation: &LoggerMutationBuilder,
    logger_states: &mut BTreeMap<String, LoggerAttachmentState>,
    handler_registry: &SharedHandlers,
    filter_registry: &SharedFilters,
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
    resolve_attachment_ids(&next, handler_registry, filter_registry)?;
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
