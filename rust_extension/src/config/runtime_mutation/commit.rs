//! Commit preparation and application for runtime mutations.

use std::{collections::BTreeMap, sync::Arc};

use pyo3::{Py, Python};

use crate::{
    config::{ConfigError, types::HandlerBuilder},
    filters::{FemtoFilter, FilterBuilder},
    logger::{FemtoLogger, HandlerAttachment},
    manager::{self, LoggerAttachmentState, RuntimeStateSnapshot},
};

use super::{LoggerScalarMutation, RuntimeConfigBuilder, SharedFilters, SharedHandlers};

pub(crate) struct BuiltRegistries {
    pub(crate) handlers: SharedHandlers,
    pub(crate) filters: SharedFilters,
    pub(crate) handler_filter_ids: BTreeMap<String, Vec<String>>,
}

pub(crate) struct RuntimeCommit {
    pub(crate) logger_states: BTreeMap<String, LoggerAttachmentState>,
    pub(crate) handler_registry: SharedHandlers,
    pub(crate) handler_filter_ids: BTreeMap<String, Vec<String>>,
    pub(crate) resolved_handler_filters: BTreeMap<String, Vec<Arc<dyn FemtoFilter>>>,
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
        let mut handler_filter_ids = before.handler_filter_ids.clone();
        handler_filter_ids.extend(built.handler_filter_ids);
        let mut filter_registry = before.filter_registry.clone();
        filter_registry.extend(built.filters);
        let resolved_handler_filters =
            resolve_handler_filters(&handler_filter_ids, &filter_registry)?;

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
            handler_filter_ids,
            resolved_handler_filters,
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
        let resolved_handlers = resolve_registered_items(
            name,
            attachment_state.handler_ids(),
            &commit.handler_registry,
            "handler",
        )?;
        let next_handlers = attachment_state
            .handler_ids()
            .iter()
            .zip(resolved_handlers)
            .map(|(handler_id, handler)| {
                let filters = commit
                    .resolved_handler_filters
                    .get(handler_id)
                    .cloned()
                    .ok_or_else(|| {
                        ConfigError::InvalidMutation(format!(
                            "{name}: missing handler filter configuration for {handler_id:?} during commit application",
                        ))
                    })?;
                Ok(HandlerAttachment::with_filters(handler, filters))
            })
            .collect::<Result<Vec<_>, ConfigError>>()?;
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
        commit.handler_filter_ids.clone(),
        commit.filter_registry.clone(),
        commit.logger_states.clone(),
    );
    Ok(())
}

fn resolve_handler_filters(
    handler_filter_ids: &BTreeMap<String, Vec<String>>,
    filter_registry: &SharedFilters,
) -> Result<BTreeMap<String, Vec<Arc<dyn FemtoFilter>>>, ConfigError> {
    handler_filter_ids
        .iter()
        .map(|(handler_id, filter_ids)| {
            let filters = filter_ids
                .iter()
                .map(|filter_id| {
                    filter_registry
                        .get(filter_id)
                        .cloned()
                        .ok_or_else(|| ConfigError::UnknownIds(vec![filter_id.clone()]))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok((handler_id.clone(), filters))
        })
        .collect()
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
