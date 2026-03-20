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
    level: Option<FemtoLevel>,
    propagate: Option<bool>,
    handlers: CollectionMutation,
    filters: CollectionMutation,
    invalid: Option<String>,
}

impl LoggerMutationBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    fn normalize_ids<I, S>(ids: I) -> Vec<String>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        ids.into_iter().map(Into::into).collect()
    }

    fn apply_ids_mutation<I, S>(
        mut self,
        ids: Option<I>,
        mutation: impl FnOnce(Option<Vec<String>>) -> CollectionMutation,
        set: impl FnOnce(&mut Self, CollectionMutation),
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let ids = ids.map(Self::normalize_ids);
        set(&mut self, mutation(ids));
        self
    }

    fn do_replace<I, S>(self, ids: I, setter: impl FnOnce(&mut Self, CollectionMutation)) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.apply_ids_mutation(
            Some(ids),
            |ids| CollectionMutation::replace(ids.unwrap_or_default()),
            setter,
        )
    }

    fn do_append<I, S>(self, ids: I, setter: impl FnOnce(&mut Self, CollectionMutation)) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.apply_ids_mutation(
            Some(ids),
            |ids| CollectionMutation::append(ids.unwrap_or_default()),
            setter,
        )
    }

    fn do_remove<I, S>(self, ids: I, setter: impl FnOnce(&mut Self, CollectionMutation)) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.apply_ids_mutation(
            Some(ids),
            |ids| CollectionMutation::remove(ids.unwrap_or_default()),
            setter,
        )
    }

    fn do_clear(self, setter: impl FnOnce(&mut Self, CollectionMutation)) -> Self {
        self.apply_ids_mutation(
            Option::<Vec<String>>::None,
            |_| CollectionMutation::Clear,
            setter,
        )
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

    fn set_handlers(&mut self, mutation: CollectionMutation) {
        if self.invalid.is_none() {
            self.invalid = collection_conflict("handlers", &self.handlers, &mutation);
        }
        self.handlers = mutation;
    }

    fn set_filters(&mut self, mutation: CollectionMutation) {
        if self.invalid.is_none() {
            self.invalid = collection_conflict("filters", &self.filters, &mutation);
        }
        self.filters = mutation;
    }

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
    handlers: BTreeMap<String, HandlerBuilder>,
    filters: BTreeMap<String, FilterBuilder>,
    loggers: BTreeMap<String, LoggerMutationBuilder>,
    root_logger: Option<LoggerMutationBuilder>,
}

pub(crate) type SharedHandlers = BTreeMap<String, Arc<dyn FemtoHandlerTrait>>;
pub(crate) type SharedFilters = BTreeMap<String, Arc<dyn FemtoFilter>>;

pub(crate) struct LoggerScalarMutation {
    pub(crate) level: Option<FemtoLevel>,
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

fn requires_existing_baseline(mutation: &CollectionMutation) -> bool {
    matches!(
        mutation,
        CollectionMutation::Append(_) | CollectionMutation::Remove(_)
    )
}

#[cfg(feature = "python")]
mod python_bindings;
