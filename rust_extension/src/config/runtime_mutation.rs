//! Runtime mutation builders and apply logic for live logger reconfiguration.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use pyo3::prelude::*;

use crate::{
    FemtoLevel,
    filters::{FemtoFilter, FilterBuilder},
    handler::FemtoHandlerTrait,
    logger::FemtoLogger,
    manager::{self, LoggerAttachmentState, RuntimeStateSnapshot},
};

use super::{
    ConfigError,
    types::{HandlerBuilder, normalize_vec},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CollectionMutation<T> {
    #[default]
    Unchanged,
    Replace(Vec<T>),
    Append(Vec<T>),
    Remove(Vec<T>),
    Clear,
}

impl CollectionMutation<String> {
    fn replace(ids: Vec<String>) -> Self {
        Self::Replace(normalize_vec(ids))
    }

    fn append(ids: Vec<String>) -> Self {
        Self::Append(normalize_vec(ids))
    }

    fn remove(ids: Vec<String>) -> Self {
        Self::Remove(normalize_vec(ids))
    }

    fn apply(&self, existing: &[String]) -> Vec<String> {
        match self {
            Self::Unchanged => existing.to_vec(),
            Self::Replace(ids) => ids.clone(),
            Self::Append(ids) => {
                let mut merged = existing.to_vec();
                let existing = merged.iter().cloned().collect::<BTreeSet<_>>();
                merged.extend(ids.iter().filter(|id| !existing.contains(*id)).cloned());
                merged
            }
            Self::Remove(ids) => {
                let removed = ids.iter().collect::<BTreeSet<_>>();
                existing
                    .iter()
                    .filter(|id| !removed.contains(*id))
                    .cloned()
                    .collect()
            }
            Self::Clear => Vec::new(),
        }
    }

    fn as_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        use pyo3::types::{PyDict, PyList};

        let dict = PyDict::new(py);
        match self {
            Self::Unchanged => {
                dict.set_item("mode", "unchanged")?;
            }
            Self::Replace(ids) => {
                dict.set_item("mode", "replace")?;
                dict.set_item("ids", PyList::new(py, ids.iter())?)?;
            }
            Self::Append(ids) => {
                dict.set_item("mode", "append")?;
                dict.set_item("ids", PyList::new(py, ids.iter())?)?;
            }
            Self::Remove(ids) => {
                dict.set_item("mode", "remove")?;
                dict.set_item("ids", PyList::new(py, ids.iter())?)?;
            }
            Self::Clear => {
                dict.set_item("mode", "clear")?;
            }
        }
        Ok(dict.unbind().into())
    }
}

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
    handlers: CollectionMutation<String>,
    filters: CollectionMutation<String>,
    invalid: Option<String>,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "public runtime-mutation builder API is exercised from Python"
    )
)]
impl LoggerMutationBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_level(mut self, level: FemtoLevel) -> Self {
        self.level = Some(level);
        self
    }

    pub fn with_propagate(mut self, propagate: bool) -> Self {
        self.propagate = Some(propagate);
        self
    }

    pub fn replace_handlers<I, S>(mut self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.set_handlers(CollectionMutation::replace(
            ids.into_iter().map(Into::into).collect(),
        ));
        self
    }

    pub fn append_handlers<I, S>(mut self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.set_handlers(CollectionMutation::append(
            ids.into_iter().map(Into::into).collect(),
        ));
        self
    }

    pub fn remove_handlers<I, S>(mut self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.set_handlers(CollectionMutation::remove(
            ids.into_iter().map(Into::into).collect(),
        ));
        self
    }

    pub fn clear_handlers(mut self) -> Self {
        self.set_handlers(CollectionMutation::Clear);
        self
    }

    pub fn replace_filters<I, S>(mut self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.set_filters(CollectionMutation::replace(
            ids.into_iter().map(Into::into).collect(),
        ));
        self
    }

    pub fn append_filters<I, S>(mut self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.set_filters(CollectionMutation::append(
            ids.into_iter().map(Into::into).collect(),
        ));
        self
    }

    pub fn remove_filters<I, S>(mut self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.set_filters(CollectionMutation::remove(
            ids.into_iter().map(Into::into).collect(),
        ));
        self
    }

    pub fn clear_filters(mut self) -> Self {
        self.set_filters(CollectionMutation::Clear);
        self
    }

    fn set_handlers(&mut self, mutation: CollectionMutation<String>) {
        self.invalid = collection_conflict("handlers", &self.handlers, &mutation);
        self.handlers = mutation;
    }

    fn set_filters(&mut self, mutation: CollectionMutation<String>) {
        self.invalid = collection_conflict("filters", &self.filters, &mutation);
        self.filters = mutation;
    }

    fn ensure_valid(&self, logger_name: &str) -> Result<(), ConfigError> {
        self.invalid
            .clone()
            .map(|message| ConfigError::InvalidMutation(format!("{logger_name}: {message}")))
            .map_or(Ok(()), Err)
    }
}

fn collection_conflict(
    kind: &str,
    current: &CollectionMutation<String>,
    new: &CollectionMutation<String>,
) -> Option<String> {
    match current {
        CollectionMutation::Unchanged => None,
        _ if current == new => None,
        _ => Some(format!("multiple {kind} mutation modes were requested")),
    }
}

/// Builder for transactional runtime reconfiguration.
///
/// `RuntimeConfigBuilder` applies handler and filter mutations against the
/// live manager state without requiring a full `ConfigBuilder.build_and_init()`
/// rebuild.
///
/// # Examples
///
/// ```rust,ignore
/// use femtologging_rs::{
///     ConfigBuilder, FemtoLevel, LoggerConfigBuilder, LoggerMutationBuilder,
///     RuntimeConfigBuilder, StreamHandlerBuilder,
/// };
///
/// ConfigBuilder::new()
///     .with_handler("stderr", StreamHandlerBuilder::stderr())
///     .with_root_logger(LoggerConfigBuilder::new().with_level(FemtoLevel::Info))
///     .build_and_init()
///     .unwrap();
///
/// RuntimeConfigBuilder::new()
///     .with_handler("stdout", StreamHandlerBuilder::stdout())
///     .with_root_logger(LoggerMutationBuilder::new().append_handlers(["stdout"]))
///     .apply()
///     .unwrap();
/// ```
#[cfg_attr(feature = "python", pyclass(from_py_object))]
#[derive(Clone, Debug, Default)]
pub struct RuntimeConfigBuilder {
    handlers: BTreeMap<String, HandlerBuilder>,
    filters: BTreeMap<String, FilterBuilder>,
    loggers: BTreeMap<String, LoggerMutationBuilder>,
    root_logger: Option<LoggerMutationBuilder>,
}

type SharedHandlers = BTreeMap<String, Arc<dyn FemtoHandlerTrait>>;
type SharedFilters = BTreeMap<String, Arc<dyn FemtoFilter>>;

struct RuntimeCommit {
    logger_states: BTreeMap<String, LoggerAttachmentState>,
    handler_registry: SharedHandlers,
    filter_registry: SharedFilters,
    impacted_loggers: Vec<(String, Py<FemtoLogger>)>,
    scalar_mutations: BTreeMap<String, LoggerMutationBuilder>,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "public runtime-mutation builder API is exercised from Python"
    )
)]
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
        let built_handlers = build_handlers(&self.handlers)?;
        let built_filters = build_filters(&self.filters)?;

        Python::attach(|py| {
            let before = manager::snapshot_runtime_state();
            let commit = self.prepare_commit(py, before.clone(), built_handlers, built_filters)?;
            apply_commit(py, &commit, before)
        })
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if let Some(root) = &self.root_logger {
            root.ensure_valid("root")?;
        }
        for (name, mutation) in &self.loggers {
            mutation.ensure_valid(name)?;
        }
        Ok(())
    }

    fn prepare_commit(
        &self,
        py: Python<'_>,
        before: RuntimeStateSnapshot,
        built_handlers: SharedHandlers,
        built_filters: SharedFilters,
    ) -> Result<RuntimeCommit, ConfigError> {
        let mut handler_registry = before.handler_registry.clone();
        handler_registry.extend(built_handlers);
        let mut filter_registry = before.filter_registry.clone();
        filter_registry.extend(built_filters);

        let mut logger_states = before.logger_states.clone();
        let mut impacted = BTreeSet::new();
        let overridden_handler_ids = self.handlers.keys().cloned().collect::<BTreeSet<_>>();
        let overridden_filter_ids = self.filters.keys().cloned().collect::<BTreeSet<_>>();

        for (name, state) in &before.logger_states {
            if state
                .handler_ids()
                .iter()
                .any(|id| overridden_handler_ids.contains(id))
                || state
                    .filter_ids()
                    .iter()
                    .any(|id| overridden_filter_ids.contains(id))
            {
                impacted.insert(name.clone());
            }
        }

        if self.root_logger.is_some() {
            impacted.insert("root".to_string());
        }
        impacted.extend(self.loggers.keys().cloned());

        if let Some(root) = &self.root_logger {
            let existing = logger_states
                .get("root")
                .cloned()
                .unwrap_or_else(LoggerAttachmentState::default);
            validate_remove_ids(existing.handler_ids(), &root.handlers)?;
            validate_remove_ids(existing.filter_ids(), &root.filters)?;
            let next = LoggerAttachmentState::new(
                root.handlers.apply(existing.handler_ids()),
                root.filters.apply(existing.filter_ids()),
            );
            resolve_attachment_ids(&next, &handler_registry, &filter_registry)?;
            logger_states.insert("root".to_string(), next);
        }

        for (name, mutation) in &self.loggers {
            let existing = logger_states
                .get(name)
                .cloned()
                .unwrap_or_else(LoggerAttachmentState::default);
            validate_remove_ids(existing.handler_ids(), &mutation.handlers)?;
            validate_remove_ids(existing.filter_ids(), &mutation.filters)?;
            let next = LoggerAttachmentState::new(
                mutation.handlers.apply(existing.handler_ids()),
                mutation.filters.apply(existing.filter_ids()),
            );
            resolve_attachment_ids(&next, &handler_registry, &filter_registry)?;
            logger_states.insert(name.clone(), next);
        }

        let impacted_loggers = impacted
            .into_iter()
            .map(|name| {
                self.fetch_impacted_logger(py, &name)
                    .map(|logger| (name, logger))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let mut scalar_mutations = BTreeMap::new();
        if let Some(root) = &self.root_logger {
            scalar_mutations.insert("root".to_string(), root.clone());
        }
        for (name, mutation) in &self.loggers {
            scalar_mutations.insert(name.clone(), mutation.clone());
        }

        Ok(RuntimeCommit {
            logger_states,
            handler_registry,
            filter_registry,
            impacted_loggers,
            scalar_mutations,
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

fn build_handlers(items: &BTreeMap<String, HandlerBuilder>) -> Result<SharedHandlers, ConfigError> {
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

fn build_filters(items: &BTreeMap<String, FilterBuilder>) -> Result<SharedFilters, ConfigError> {
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

fn resolve_attachment_ids(
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

fn validate_remove_ids(
    existing: &[String],
    mutation: &CollectionMutation<String>,
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

fn apply_commit(
    py: Python<'_>,
    commit: &RuntimeCommit,
    before: RuntimeStateSnapshot,
) -> Result<(), ConfigError> {
    let snapshots = commit
        .impacted_loggers
        .iter()
        .map(|(name, logger)| {
            let snapshot = logger.borrow(py).snapshot_runtime_state();
            (name.clone(), logger.clone_ref(py), snapshot)
        })
        .collect::<Vec<_>>();

    let result = {
        for (name, logger) in &commit.impacted_loggers {
            let logger_ref = logger.borrow(py);
            let attachment_state = commit
                .logger_states
                .get(name)
                .cloned()
                .unwrap_or_else(LoggerAttachmentState::default);
            let next_handlers = attachment_state
                .handler_ids()
                .iter()
                .map(|id| {
                    commit
                        .handler_registry
                        .get(id)
                        .cloned()
                        .expect("validated handler registry lookup")
                })
                .collect::<Vec<_>>();
            let next_filters = attachment_state
                .filter_ids()
                .iter()
                .map(|id| {
                    commit
                        .filter_registry
                        .get(id)
                        .cloned()
                        .expect("validated filter registry lookup")
                })
                .collect::<Vec<_>>();
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
    };

    if result.is_err() {
        for (_, logger, snapshot) in snapshots {
            logger.borrow(py).restore_runtime_state(&snapshot);
        }
        manager::replace_runtime_state(
            before.handler_registry,
            before.filter_registry,
            before.logger_states,
        );
    }

    result
}

fn apply_scalar_mutation(
    logger_ref: &pyo3::PyRef<'_, FemtoLogger>,
    mutation: &LoggerMutationBuilder,
) {
    if let Some(level) = mutation.level {
        logger_ref.set_level(level);
    }
    if let Some(propagate) = mutation.propagate {
        logger_ref.set_propagate(propagate);
    }
}

#[cfg(feature = "python")]
mod python_bindings;
