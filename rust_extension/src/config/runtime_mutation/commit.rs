//! Commit preparation and application for runtime mutations.

use std::{collections::BTreeMap, sync::Arc};

use pyo3::{Py, Python};

use crate::{
    config::{ConfigError, types::HandlerBuilder},
    filters::FilterBuilder,
    logger::FemtoLogger,
    manager::{self, LoggerAttachmentState, RuntimeStateSnapshot},
};

use super::{LoggerScalarMutation, RuntimeConfigBuilder, SharedFilters, SharedHandlers};

pub(crate) struct BuiltRegistries {
    pub(crate) handlers: SharedHandlers,
    pub(crate) filters: SharedFilters,
}

pub(crate) struct RuntimeCommit {
    pub(crate) logger_states: BTreeMap<String, LoggerAttachmentState>,
    pub(crate) handler_registry: SharedHandlers,
    pub(crate) filter_registry: SharedFilters,
    pub(crate) impacted_loggers: Vec<(String, Py<FemtoLogger>)>,
    pub(crate) scalar_mutations: BTreeMap<String, LoggerScalarMutation>,
}

impl RuntimeConfigBuilder {
    pub(crate) fn prepare_commit(
        &self,
        py: Python<'_>,
        before: RuntimeStateSnapshot,
        built: BuiltRegistries,
    ) -> Result<RuntimeCommit, ConfigError> {
        let mut handler_registry = before.handler_registry.clone();
        handler_registry.extend(built.handlers);
        let mut filter_registry = before.filter_registry.clone();
        filter_registry.extend(built.filters);

        let mut logger_states = before.logger_states.clone();
        let impacted = self.collect_impacted(&before);

        self.apply_logger_mutations(&mut logger_states, &handler_registry, &filter_registry)?;

        let impacted_loggers = impacted
            .into_iter()
            .map(|name| {
                self.fetch_impacted_logger(py, &name)
                    .map(|logger| (name, logger))
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(RuntimeCommit {
            logger_states,
            handler_registry,
            filter_registry,
            impacted_loggers,
            scalar_mutations: self.build_scalar_mutations(),
        })
    }

    fn fetch_impacted_logger(
        &self,
        py: Python<'_>,
        name: &str,
    ) -> Result<Py<FemtoLogger>, ConfigError> {
        manager::get_logger(py, name)
            .map_err(|err| ConfigError::LoggerInit(format!("{name}: {err}")))
    }
}

pub(crate) fn build_handlers(
    items: &BTreeMap<String, HandlerBuilder>,
) -> Result<SharedHandlers, ConfigError> {
    let mut built = BTreeMap::new();
    for (id, builder) in items {
        let handler = builder
            .build()
            .map_err(|source| ConfigError::HandlerBuild {
                id: id.clone(),
                source,
            })?;
        built.insert(id.clone(), handler);
    }
    Ok(built)
}

pub(crate) fn build_filters(
    items: &BTreeMap<String, FilterBuilder>,
) -> Result<SharedFilters, ConfigError> {
    let mut built = BTreeMap::new();
    for (id, builder) in items {
        let filter = builder.build().map_err(|source| ConfigError::FilterBuild {
            id: id.clone(),
            source,
        })?;
        built.insert(id.clone(), filter);
    }
    Ok(built)
}

pub(crate) fn apply_commit(py: Python<'_>, commit: &RuntimeCommit) -> Result<(), ConfigError> {
    for (name, logger) in &commit.impacted_loggers {
        let logger_ref = logger.borrow(py);
        let attachment_state = commit
            .logger_states
            .get(name)
            .cloned()
            .unwrap_or_else(LoggerAttachmentState::default);
        let next_handlers = resolve_registered_items(
            name,
            attachment_state.handler_ids(),
            &commit.handler_registry,
            "handler",
        )?;
        let next_filters = resolve_registered_items(
            name,
            attachment_state.filter_ids(),
            &commit.filter_registry,
            "filter",
        )?;
        logger_ref.replace_handlers(next_handlers);
        logger_ref.replace_filters(next_filters);
        if let Some(mutation) = commit.scalar_mutations.get(name) {
            apply_scalar_mutation(&logger_ref, mutation);
        }
    }
    manager::replace_runtime_state(
        commit.handler_registry.clone(),
        commit.filter_registry.clone(),
        commit.logger_states.clone(),
    );
    Ok(())
}

fn apply_scalar_mutation(
    logger_ref: &pyo3::PyRef<'_, FemtoLogger>,
    mutation: &LoggerScalarMutation,
) {
    if let Some(level) = mutation.level {
        logger_ref.set_level(level);
    }
    if let Some(propagate) = mutation.propagate {
        logger_ref.set_propagate(propagate);
    }
}

fn resolve_registered_items<T: ?Sized>(
    logger_name: &str,
    ids: &[String],
    registry: &BTreeMap<String, Arc<T>>,
    kind: &str,
) -> Result<Vec<Arc<T>>, ConfigError> {
    ids.iter()
        .map(|id| {
            registry.get(id).cloned().ok_or_else(|| {
                ConfigError::InvalidMutation(format!(
                    "{logger_name}: missing {kind} attachment id {id:?} during commit application",
                ))
            })
        })
        .collect()
}
