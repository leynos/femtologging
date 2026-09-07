//! Construction and realization of configuration.

use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use pyo3::prelude::*;

use crate::config::ConfigError;
use crate::{
    filters::{FemtoFilter, FilterBuilder},
    handler::FemtoHandlerTrait,
    level::FemtoLevel,
    logger::FemtoLogger,
    manager,
};

use super::types::{ConfigBuilder, HandlerBuilder, LoggerConfigBuilder};

struct ConfiguredLoggerPlan {
    name: String,
    logger: Py<FemtoLogger>,
    handler_ids: Vec<String>,
    filter_ids: Vec<String>,
    handlers: Vec<Arc<dyn FemtoHandlerTrait>>,
    filters: Vec<Arc<dyn FemtoFilter>>,
    level: Option<FemtoLevel>,
    propagate: Option<bool>,
}

struct LoggerPlanSources<'a> {
    handlers: &'a BTreeMap<String, Arc<dyn FemtoHandlerTrait>>,
    filters: &'a BTreeMap<String, Arc<dyn FemtoFilter>>,
    default_level: Option<FemtoLevel>,
}

impl ConfigBuilder {
    /// Finalize the configuration and initialize loggers.
    ///
    /// # Errors
    ///
    /// Returns an error when the schema version or root logger is invalid, a
    /// handler or filter cannot be built, or a logger cannot be initialized.
    pub fn build_and_init(&self) -> Result<(), ConfigError> {
        if self.version() != 1 {
            return Err(ConfigError::UnsupportedVersion(self.version()));
        }
        if self.root_logger().is_none() {
            return Err(ConfigError::MissingRootLogger);
        }
        let built_handlers = Self::build_map(
            self.handler_builders(),
            HandlerBuilder::build,
            |id, source| ConfigError::HandlerBuild { id, source },
        )?;
        let built_filters = Self::build_map(
            self.filter_builders(),
            FilterBuilder::build,
            |id, source| ConfigError::FilterBuild { id, source },
        )?;

        Python::attach(|py| -> Result<_, ConfigError> {
            if self.disable_existing_loggers() {
                let mut keep_names: HashSet<String> = self
                    .logger_builders()
                    .keys()
                    .cloned()
                    .chain(std::iter::once(String::from("root")))
                    .collect();
                self.extend_keep_names_with_ancestors(&mut keep_names);
                manager::disable_existing_loggers(py, &keep_names)
                    .map_err(|e| ConfigError::LoggerInit(e.to_string()))?;
            }

            let targets = self
                .root_logger()
                .map(|c| ("root", c))
                .into_iter()
                .chain(self.logger_builders().iter().map(|(n, c)| (n.as_str(), c)));
            let plans = targets
                .map(|(name, cfg)| {
                    Self::prepare_logger_plan(
                        py,
                        name,
                        cfg,
                        &LoggerPlanSources {
                            handlers: &built_handlers,
                            filters: &built_filters,
                            default_level: self.default_level(),
                        },
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;

            let mut runtime_loggers = BTreeMap::new();
            for plan in &plans {
                Self::apply_logger_plan(py, plan);
                runtime_loggers.insert(
                    plan.name.clone(),
                    manager::LoggerAttachmentState::new(
                        plan.handler_ids.clone(),
                        plan.filter_ids.clone(),
                    ),
                );
            }
            manager::replace_runtime_state(
                built_handlers.clone(),
                built_filters.clone(),
                runtime_loggers,
            );
            Ok(())
        })?;
        Ok(())
    }

    /// Include ancestors of each configured logger (e.g., `a.b.c` keeps `a.b`
    /// and `a`) in the keep set used by `disable_existing_loggers`.
    fn extend_keep_names_with_ancestors(&self, keep_names: &mut HashSet<String>) {
        for name in self.logger_builders().keys() {
            Self::insert_logger_ancestors(name, keep_names);
        }
    }

    fn insert_logger_ancestors(name: &str, keep_names: &mut HashSet<String>) {
        let mut cur = name;
        while let Some((parent, _)) = cur.rsplit_once('.') {
            keep_names.insert(parent.to_owned());
            cur = parent;
        }
    }

    fn build_map<B, O, E, F, G>(
        items: &BTreeMap<String, B>,
        mut builder_fn: F,
        wrap_err: G,
    ) -> Result<BTreeMap<String, O>, ConfigError>
    where
        F: FnMut(&B) -> Result<O, E>,
        G: Fn(String, E) -> ConfigError,
    {
        let mut result = BTreeMap::new();
        for (id, builder) in items {
            let obj = builder_fn(builder).map_err(|e| wrap_err(id.clone(), e))?;
            result.insert(id.clone(), obj);
        }
        Ok(result)
    }

    fn fetch_logger(py: Python<'_>, name: &str) -> Result<Py<FemtoLogger>, ConfigError> {
        manager::get_logger(py, name).map_err(|e| ConfigError::LoggerInit(format!("{name}: {e}")))
    }

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

    const fn duplicate_handler_ids(ids: Vec<String>) -> ConfigError {
        ConfigError::DuplicateHandlerIds(ids)
    }

    const fn duplicate_filter_ids(ids: Vec<String>) -> ConfigError {
        ConfigError::DuplicateFilterIds(ids)
    }

    fn prepare_logger_plan(
        py: Python<'_>,
        name: &str,
        cfg: &LoggerConfigBuilder,
        sources: &LoggerPlanSources<'_>,
    ) -> Result<ConfiguredLoggerPlan, ConfigError> {
        let logger = Self::fetch_logger(py, name)?;
        let planned_handlers = Self::collect_items(
            cfg.handler_ids(),
            sources.handlers,
            Self::duplicate_handler_ids,
        )?;
        let planned_filters = Self::collect_items(
            cfg.filter_ids(),
            sources.filters,
            Self::duplicate_filter_ids,
        )?;
        Ok(ConfiguredLoggerPlan {
            name: name.to_owned(),
            logger,
            handler_ids: cfg.handler_ids().to_vec(),
            filter_ids: cfg.filter_ids().to_vec(),
            handlers: planned_handlers,
            filters: planned_filters,
            level: cfg.level_opt().or(sources.default_level),
            propagate: cfg.propagate_opt(),
        })
    }

    fn apply_logger_plan(py: Python<'_>, plan: &ConfiguredLoggerPlan) {
        let logger_ref = plan.logger.borrow(py);
        logger_ref.clear_handlers();
        for handler in &plan.handlers {
            logger_ref.add_handler(handler.clone());
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
    }
}
