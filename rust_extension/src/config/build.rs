#![cfg(feature = "python")]
//! Construction and realisation of configuration.

use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use pyo3::prelude::*;

use crate::config::ConfigError;
use crate::{filters::FemtoFilter, handler::FemtoHandlerTrait, logger::FemtoLogger, manager};

use super::types::{ConfigBuilder, LoggerConfigBuilder};

impl ConfigBuilder {
    /// Finalise the configuration and initialise loggers.
    pub fn build_and_init(&self) -> Result<(), ConfigError> {
        if self.version != 1 {
            return Err(ConfigError::UnsupportedVersion(self.version));
        }
        if self.root_logger.is_none() {
            return Err(ConfigError::MissingRootLogger);
        }
        let built_handlers = Self::build_map(
            &self.handlers,
            |b| b.build(),
            |id, source| ConfigError::HandlerBuild { id, source },
        )?;
        let built_filters = Self::build_map(
            &self.filters,
            |b| b.build(),
            |id, source| ConfigError::FilterBuild { id, source },
        )?;

        Python::with_gil(|py| -> Result<_, ConfigError> {
            // Handle disable_existing_loggers if requested
            if self.disable_existing_loggers {
                let mut keep_names: HashSet<String> = self
                    .loggers
                    .keys()
                    .cloned()
                    .chain(std::iter::once("root".to_string()))
                    .collect();
                // Include ancestors of each kept logger (e.g., "a.b.c" keeps "a.b" and "a").
                for name in self.loggers.keys() {
                    let mut cur = name.as_str();
                    while let Some((parent, _)) = cur.rsplit_once('.') {
                        keep_names.insert(parent.to_string());
                        cur = parent;
                    }
                }
                manager::disable_existing_loggers(py, &keep_names)
                    .map_err(|e| ConfigError::LoggerInit(e.to_string()))?;
            }

            let targets = self
                .root_logger
                .as_ref()
                .map(|c| ("root", c))
                .into_iter()
                .chain(self.loggers.iter().map(|(n, c)| (n.as_str(), c)));

            for (name, cfg) in targets {
                let logger = self.fetch_logger(py, name)?;
                self.apply_logger_config(py, &logger, cfg, &built_handlers, &built_filters)?;
            }
            Ok(())
        })?;
        Ok(())
    }

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

    fn fetch_logger<'py>(
        &self,
        py: Python<'py>,
        name: &str,
    ) -> Result<Py<FemtoLogger>, ConfigError> {
        manager::get_logger(py, name).map_err(|e| ConfigError::LoggerInit(format!("{name}: {e}")))
    }

    fn apply_logger_config<'py>(
        &self,
        py: Python<'py>,
        logger: &Py<FemtoLogger>,
        cfg: &LoggerConfigBuilder,
        handlers: &BTreeMap<String, Arc<dyn FemtoHandlerTrait>>,
        filters: &BTreeMap<String, Arc<dyn FemtoFilter>>,
    ) -> Result<(), ConfigError> {
        let logger_ref = logger.borrow(py);
        self.apply_handlers(&logger_ref, cfg.handler_ids(), handlers)?;
        self.apply_filters(&logger_ref, cfg.filter_ids(), filters)?;
        if let Some(level) = cfg.level_opt() {
            logger_ref.set_level(level);
        }
        Ok(())
    }
    /// Collect items for later application to a logger.
    ///
    /// Deduplicates `ids`, raising `dup_err` if any repeats are found, and
    /// returns the objects fetched from `pool`. Callers should clear existing
    /// logger state and attach the returned items.
    fn collect_items<T: ?Sized>(
        ids: &[String],
        pool: &BTreeMap<String, Arc<T>>,
        dup_err: impl FnOnce(Vec<String>) -> ConfigError,
    ) -> Result<Vec<Arc<T>>, ConfigError> {
        let mut seen = HashSet::with_capacity(ids.len());
        let mut dup = Vec::new();
        let mut items = Vec::with_capacity(ids.len());
        for id in ids {
            if !seen.insert(id) {
                dup.push(id.clone());
                continue;
            }
            let obj = pool
                .get(id)
                .cloned()
                .ok_or_else(|| ConfigError::UnknownId(id.clone()))?;
            items.push(obj);
        }
        if !dup.is_empty() {
            return Err(dup_err(dup));
        }
        Ok(items)
    }

    fn apply_handlers(
        &self,
        logger_ref: &PyRef<FemtoLogger>,
        ids: &[String],
        pool: &BTreeMap<String, Arc<dyn FemtoHandlerTrait>>,
    ) -> Result<(), ConfigError> {
        let items = Self::collect_items(ids, pool, ConfigError::DuplicateHandlerIds)?;
        logger_ref.clear_handlers();
        for h in items {
            logger_ref.add_handler(h);
        }
        Ok(())
    }

    fn apply_filters(
        &self,
        logger_ref: &PyRef<FemtoLogger>,
        ids: &[String],
        pool: &BTreeMap<String, Arc<dyn FemtoFilter>>,
    ) -> Result<(), ConfigError> {
        let items = Self::collect_items(ids, pool, ConfigError::DuplicateFilterIds)?;
        logger_ref.clear_filters();
        for f in items {
            logger_ref.add_filter(f);
        }
        Ok(())
    }
}
