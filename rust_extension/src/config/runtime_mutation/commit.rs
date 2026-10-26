//! Commit preparation and application for runtime mutations.

use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use pyo3::{Py, Python};

use crate::{
    config::{ConfigError, types::HandlerBuilder},
    filters::{FemtoFilter, FilterBuilder},
    logger::{FemtoLogger, HandlerAttachment},
    manager::{self, LoggerAttachmentState, RuntimeStateSnapshot},
};

use super::{LoggerScalarMutation, RuntimeConfigBuilder, SharedFilters, SharedHandlers};

/// Built handler and filter registries ready to merge into the runtime snapshot.
pub(crate) struct BuiltRegistries {
    /// Newly built handlers keyed by their requested IDs.
    pub(crate) handlers: SharedHandlers,
    /// Newly built filters keyed by their requested IDs.
    pub(crate) filters: SharedFilters,
    /// Filter IDs configured on each newly built handler.
    pub(crate) handler_filter_ids: BTreeMap<String, Vec<String>>,
}

/// Fully prepared state applied as one runtime mutation commit.
pub(crate) struct RuntimeCommit {
    /// Logger attachment states after applying requested collection mutations.
    pub(crate) logger_states: BTreeMap<String, LoggerAttachmentState>,
    /// Handler registry after merging newly built handlers with the snapshot.
    pub(crate) handler_registry: SharedHandlers,
    /// Filter IDs retained for each registered handler.
    pub(crate) handler_filter_ids: BTreeMap<String, Vec<String>>,
    /// Resolved filters applied to each handler before producer dispatch.
    pub(crate) resolved_handler_filters: BTreeMap<String, Vec<Arc<dyn FemtoFilter>>>,
    /// Filter registry after merging newly built filters with the snapshot.
    pub(crate) filter_registry: SharedFilters,
    /// Python logger objects whose live attachments and scalar fields must be updated.
    pub(crate) impacted_loggers: Vec<(String, Py<FemtoLogger>)>,
    /// Scalar overrides to apply to the impacted logger objects.
    pub(crate) scalar_mutations: BTreeMap<String, LoggerScalarMutation>,
}

impl RuntimeConfigBuilder {
    /// Builds a private, validated commit from the runtime snapshot and requested changes.
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

        // A mutation may name a logger that does not exist yet, and
        // `get_logger` registers it. This preflight narrows the chance of a
        // partial registration, but each lookup takes the registry lock
        // separately, so a concurrent writer can consume the capacity first.
        // A late refusal can therefore leave earlier fetched loggers present.
        let requested = impacted.iter().cloned().collect::<HashSet<_>>();
        manager::ensure_registry_capacity(&requested)
            .map_err(|error| ConfigError::LoggerInit(error.to_string()))?;

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

    /// Looks up an impacted Python logger without mutating it, mapping lookup failures to `LoggerInit`.
    fn fetch_impacted_logger(
        &self,
        py: Python<'_>,
        name: &str,
    ) -> Result<Py<FemtoLogger>, ConfigError> {
        manager::get_logger(py, name)
            .map_err(|err| ConfigError::LoggerInit(format!("{name}: {err}")))
    }
}

/// Resolved values needed to apply one logger update without further validation.
struct ResolvedLoggerUpdate<'a> {
    /// Python logger whose live state is being replaced.
    logger: &'a Py<FemtoLogger>,
    /// Fully resolved handlers for the logger.
    handlers: Vec<HandlerAttachment>,
    /// Fully resolved logger filters.
    filters: Vec<Arc<dyn FemtoFilter>>,
    /// Optional scalar changes prepared for this logger.
    scalar_mutation: Option<&'a LoggerScalarMutation>,
}

/// Builds requested handlers, preserving each identifier in a `HandlerBuild` error on failure.
pub(crate) fn build_handlers(
    items: &BTreeMap<String, HandlerBuilder>,
) -> Result<SharedHandlers, ConfigError> {
    build_registry(items, HandlerBuilder::build, |id, source| {
        ConfigError::HandlerBuild { id, source }
    })
}

/// Builds requested filters, preserving each identifier in a `FilterBuild` error on failure.
pub(crate) fn build_filters(
    items: &BTreeMap<String, FilterBuilder>,
) -> Result<SharedFilters, ConfigError> {
    build_registry(items, FilterBuilder::build, |id, source| {
        ConfigError::FilterBuild { id, source }
    })
}

/// Builds each registry entry and preserves its ID in the domain-specific error.
fn build_registry<Builder, Value, BuildError>(
    items: &BTreeMap<String, Builder>,
    mut build: impl FnMut(&Builder) -> Result<Value, BuildError>,
    mut map_error: impl FnMut(String, BuildError) -> ConfigError,
) -> Result<BTreeMap<String, Value>, ConfigError> {
    let mut built = BTreeMap::new();
    for (id, builder) in items {
        let value = build(builder).map_err(|source| map_error(id.clone(), source))?;
        built.insert(id.clone(), value);
    }
    Ok(built)
}

/// Publishes live logger attachments and the registry snapshot after validation succeeds.
pub(crate) fn apply_commit(py: Python<'_>, commit: &RuntimeCommit) -> Result<(), ConfigError> {
    let logger_updates = commit
        .impacted_loggers
        .iter()
        .map(|(name, logger)| resolve_logger_commit(name, logger, commit))
        .collect::<Result<Vec<_>, _>>()?;

    let mut displaced = manager::DisplacedAttachments::default();
    for update in logger_updates {
        apply_logger_commit(py, update, &mut displaced);
    }

    displaced.runtime_state = Some(manager::replace_runtime_state(
        commit.handler_registry.clone(),
        commit.handler_filter_ids.clone(),
        commit.filter_registry.clone(),
        commit.logger_states.clone(),
    ));
    displaced.drop_without_gil(py);
    Ok(())
}

/// Resolve a logger's next attachments and scalar changes before any logger is mutated.
fn resolve_logger_commit<'a>(
    name: &str,
    logger: &'a Py<FemtoLogger>,
    commit: &'a RuntimeCommit,
) -> Result<ResolvedLoggerUpdate<'a>, ConfigError> {
    let attachment_state = commit.logger_states.get(name).cloned().unwrap_or_default();
    let resolved_handlers = resolve_registered_items(
        name,
        attachment_state.handler_ids(),
        &commit.handler_registry,
        "handler",
    )?;
    let handlers = attachment_state
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
    let filters = resolve_registered_items(
        name,
        attachment_state.filter_ids(),
        &commit.filter_registry,
        "filter",
    )?;

    Ok(ResolvedLoggerUpdate {
        logger,
        handlers,
        filters,
        scalar_mutation: commit.scalar_mutations.get(name),
    })
}

/// Apply a fully resolved logger update; this phase contains no fallible lookups.
fn apply_logger_commit(
    py: Python<'_>,
    update: ResolvedLoggerUpdate<'_>,
    displaced: &mut manager::DisplacedAttachments,
) {
    let logger_ref = update.logger.borrow(py);
    let mut old_handlers = logger_ref.replace_handlers(update.handlers);
    displaced.handlers.append(&mut old_handlers);
    let mut old_filters = logger_ref.replace_filters(update.filters);
    displaced.filters.append(&mut old_filters);
    if let Some(mutation) = update.scalar_mutation {
        apply_scalar_mutation(&logger_ref, mutation);
    }
}

/// Resolves each handler's configured filter IDs against the merged filter registry.
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

/// Applies only the scalar fields supplied by a logger mutation.
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

/// Resolves attachment IDs from a registry and reports a logger-specific missing-ID error.
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

#[cfg(test)]
#[path = "commit_tests.rs"]
mod tests;
