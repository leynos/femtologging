//! Construction and realisation of configuration.

use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use pyo3::prelude::*;

use crate::config::ConfigError;
use crate::{
    filters::FemtoFilter,
    handler::FemtoHandlerTrait,
    level::FemtoLevel,
    logger::{FemtoLogger, HandlerAttachment},
    manager,
};

use super::types::{ConfigBuilder, LoggerConfigBuilder};

/// A validated logger mutation kept detached from the global manager.
///
/// Planning resolves handlers and filters before any live logger is changed;
/// committing the matching logger objects is a distinct lifecycle step.
struct ConfiguredLoggerPlan {
    /// Registry name used to match this plan with its committed logger.
    name: String,
    /// Handler identities retained in the runtime snapshot for later updates.
    handler_ids: Vec<String>,
    /// Filter identities retained in the runtime snapshot for later updates.
    filter_ids: Vec<String>,
    /// Shared handlers already validated against this configuration.
    handlers: Vec<HandlerAttachment>,
    /// Shared filters already validated against this configuration.
    filters: Vec<Arc<dyn FemtoFilter>>,
    /// Optional threshold that deliberately leaves the existing level intact.
    level: Option<FemtoLevel>,
    /// Optional propagation choice that deliberately leaves it intact when absent.
    propagate: Option<bool>,
}

/// Runtime-ready registries prepared before logger state is committed.
struct BuiltConfigComponents {
    /// Handler instances keyed by their configuration identifiers.
    built_handlers: BTreeMap<String, Arc<dyn FemtoHandlerTrait>>,
    /// Original filter references retained for later runtime replacement.
    handler_filter_ids: BTreeMap<String, Vec<String>>,
    /// Filter instances keyed by their configuration identifiers.
    built_filters: BTreeMap<String, Arc<dyn FemtoFilter>>,
    /// Resolved filter chains keyed by handler identifier, in configured order.
    resolved_handler_filters: BTreeMap<String, Vec<Arc<dyn FemtoFilter>>>,
}
impl ConfigBuilder {
    /// Finalize the configuration and initialize loggers.
    pub fn build_and_init(&self) -> Result<(), ConfigError> {
        if self.version() != 1 {
            return Err(ConfigError::UnsupportedVersion(self.version()));
        }
        if self.root_logger().is_none() {
            return Err(ConfigError::MissingRootLogger);
        }
        let components = self.build_runtime_components()?;

        Python::attach(|py| -> Result<_, ConfigError> {
            let BuiltConfigComponents {
                built_handlers,
                handler_filter_ids,
                built_filters,
                resolved_handler_filters: _,
            } = &components;
            let targets = self
                .root_logger()
                .map(|c| ("root", c))
                .into_iter()
                .chain(self.logger_builders().iter().map(|(n, c)| (n.as_str(), c)));
            let plans = targets
                .map(|(name, cfg)| self.prepare_logger_plan(name, cfg, &components))
                .collect::<Result<Vec<_>, _>>()?;
            let staged_loggers =
                manager::stage_loggers(py, plans.iter().map(|plan| plan.name.as_str()))
                    .map_err(|error| ConfigError::LoggerInit(error.to_string()))?;
            let keep_names = self.disable_existing_loggers().then(|| {
                let mut names: HashSet<String> = self
                    .logger_builders()
                    .keys()
                    .cloned()
                    .chain(std::iter::once("root".to_string()))
                    .collect();
                self.extend_keep_names_with_ancestors(&mut names);
                names
            });
            let committed_loggers = manager::commit_staged_loggers(py, staged_loggers);

            if let Some(keep_names) = &keep_names {
                manager::disable_existing_loggers(py, keep_names);
            }
            let runtime_loggers =
                self.apply_committed_logger_plans(py, &plans, &committed_loggers)?;
            manager::replace_runtime_state(
                (*built_handlers).clone(),
                (*handler_filter_ids).clone(),
                (*built_filters).clone(),
                runtime_loggers,
            );
            Ok(())
        })?;
        Ok(())
    }

    /// Attach validated plans only after every logger has been committed.
    ///
    /// This preserves configuration atomicity: a failed plan cannot leave a
    /// newly created logger in the manager with partially attached resources.
    fn apply_committed_logger_plans(
        &self,
        py: Python<'_>,
        plans: &[ConfiguredLoggerPlan],
        committed_loggers: &BTreeMap<String, Py<FemtoLogger>>,
    ) -> Result<BTreeMap<String, manager::LoggerAttachmentState>, ConfigError> {
        let mut runtime_loggers = BTreeMap::new();
        for plan in plans {
            let logger = committed_loggers.get(&plan.name).ok_or_else(|| {
                ConfigError::LoggerInit(format!(
                    "missing committed logger for configuration plan {}",
                    plan.name
                ))
            })?;
            self.apply_logger_plan(py, logger, plan)?;
            runtime_loggers.insert(
                plan.name.clone(),
                manager::LoggerAttachmentState::new(
                    plan.handler_ids.clone(),
                    plan.filter_ids.clone(),
                ),
            );
        }
        Ok(runtime_loggers)
    }

    /// Include ancestors of each configured logger (e.g., `a.b.c` keeps `a.b`
    /// and `a`) in the keep set used by `disable_existing_loggers`.
    fn extend_keep_names_with_ancestors(&self, keep_names: &mut HashSet<String>) {
        for name in self.logger_builders().keys() {
            Self::insert_logger_ancestors(name, keep_names);
        }
    }

    /// Add each dotted parent so disabling old loggers cannot break propagation.
    fn insert_logger_ancestors(name: &str, keep_names: &mut HashSet<String>) {
        let mut cur = name;
        while let Some((parent, _)) = cur.rsplit_once('.') {
            keep_names.insert(parent.to_string());
            cur = parent;
        }
    }

    /// Build a named registry while preserving the failing configuration ID.
    fn build_map<B, O, E, F, G>(
        items: &BTreeMap<String, B>,
        mut build: F,
        wrap_err: G,
    ) -> Result<BTreeMap<String, O>, ConfigError>
    where
        F: FnMut(&B) -> Result<O, E>,
        G: Fn(String, E) -> ConfigError,
    {
        let mut built = BTreeMap::new();
        for (id, builder) in items {
            let obj = build(builder).map_err(|e| wrap_err(id.clone(), e))?;
            built.insert(id.clone(), obj);
        }
        Ok(built)
    }

    /// Resolve referenced resources once and reject duplicate or unknown IDs.
    fn collect_items<T: ?Sized>(
        ids: &[String],
        pool: &BTreeMap<String, Arc<T>>,
        dup_err: impl FnOnce(Vec<String>) -> ConfigError,
    ) -> Result<Vec<Arc<T>>, ConfigError> {
        let mut seen = HashSet::new();
        let mut dup = Vec::new();
        let mut missing = Vec::new();
        let mut items = Vec::new();

        for id in ids {
            if !seen.insert(id.clone()) {
                dup.push(id.clone());
                continue;
            }
            match pool.get(id) {
                Some(item) => items.push(item.clone()),
                None => missing.push(id.clone()),
            }
        }

        if !dup.is_empty() {
            return Err(dup_err(dup));
        }
        if !missing.is_empty() {
            return Err(ConfigError::UnknownIds(missing));
        }
        Ok(items)
    }

    /// Convert repeated handler references into the configuration error type.
    fn duplicate_handler_ids(ids: Vec<String>) -> ConfigError {
        ConfigError::DuplicateHandlerIds(ids)
    }

    /// Convert repeated filter references into the configuration error type.
    fn duplicate_filter_ids(ids: Vec<String>) -> ConfigError {
        ConfigError::DuplicateFilterIds(ids)
    }

    /// Validate a logger's resource references without changing live state.
    fn prepare_logger_plan(
        &self,
        name: &str,
        cfg: &LoggerConfigBuilder,
        components: &BuiltConfigComponents,
    ) -> Result<ConfiguredLoggerPlan, ConfigError> {
        let resolved_handlers = Self::collect_items(
            cfg.handler_ids(),
            &components.built_handlers,
            Self::duplicate_handler_ids,
        )?;
        #[cfg(feature = "python")]
        if resolved_handlers
            .iter()
            .any(|handler| handler.is_python_backed() && !handler.provides_context_dispatch())
        {
            return Err(ConfigError::LoggerInit(
                "Python-backed handlers must provide contextual dispatch".to_owned(),
            ));
        }
        let planned_handlers = cfg
            .handler_ids()
            .iter()
            .zip(resolved_handlers)
            .map(|(id, handler)| {
                let filters = components
                    .resolved_handler_filters
                    .get(id)
                    .ok_or_else(|| ConfigError::UnknownIds(vec![id.clone()]))?;
                Ok(HandlerAttachment::with_filters(handler, filters.clone()))
            })
            .collect::<Result<Vec<_>, ConfigError>>()?;
        let planned_filters = Self::collect_items(
            cfg.filter_ids(),
            &components.built_filters,
            Self::duplicate_filter_ids,
        )?;
        Ok(ConfiguredLoggerPlan {
            name: name.to_string(),
            handler_ids: cfg.handler_ids().to_vec(),
            filter_ids: cfg.filter_ids().to_vec(),
            handlers: planned_handlers,
            filters: planned_filters,
            level: cfg.level_opt().or(self.default_level()),
            propagate: cfg.propagate_opt(),
        })
    }

    /// Replace a committed logger's attachments while the Python GIL is held.
    ///
    /// Handler and filter objects are shared by `Arc`; borrowing the PyO3
    /// logger here confines Python object access to the attached interpreter.
    fn apply_logger_plan(
        &self,
        py: Python<'_>,
        logger: &Py<FemtoLogger>,
        plan: &ConfiguredLoggerPlan,
    ) -> Result<(), ConfigError> {
        let logger_ref = logger.borrow(py);
        logger_ref.clear_handlers();
        for handler in &plan.handlers {
            logger_ref
                .add_handler_with_filters(handler.handler.clone(), handler.filters.clone())
                .map_err(|error| ConfigError::LoggerInit(error.to_string()))?;
        }
        logger_ref.clear_filters();
        for filter in &plan.filters {
            logger_ref.add_filter(filter.clone());
        }
        if let Some(level) = plan.level {
            logger_ref.set_level(level);
        }
        if let Some(propagate) = plan.propagate {
            logger_ref.set_propagate(propagate);
        }
        Ok(())
    }

    /// Build formatter, filter, and handler registries before touching live loggers.
    fn build_runtime_components(&self) -> Result<BuiltConfigComponents, ConfigError> {
        let built_formatters = self
            .formatter_builders()
            .iter()
            .map(|(id, builder)| (id.clone(), builder.build()))
            .collect::<BTreeMap<_, _>>();
        let built_filters = Self::build_map(
            self.filter_builders(),
            |b| b.build(),
            |id, source| ConfigError::FilterBuild { id, source },
        )?;
        let built_handlers = Self::build_map(
            self.handler_builders(),
            |b| b.build_with_formatters(&built_formatters),
            |id, source| ConfigError::HandlerBuild { id, source },
        )?;
        let handler_filters = self
            .handler_builders()
            .iter()
            .map(|(id, builder)| {
                Self::collect_items(
                    builder.filter_ids(),
                    &built_filters,
                    Self::duplicate_filter_ids,
                )
                .map(|filters| (id.clone(), filters))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let handler_filter_ids = self
            .handler_builders()
            .iter()
            .map(|(id, builder)| (id.clone(), builder.filter_ids().to_vec()))
            .collect();

        Ok(BuiltConfigComponents {
            built_handlers,
            handler_filter_ids,
            built_filters,
            resolved_handler_filters: handler_filters,
        })
    }
}
