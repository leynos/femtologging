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
        self.apply_items(
            &logger_ref,                       // logger to mutate
            cfg.handler_ids(),                 // declared handler identifiers
            handlers,                          // pool of built handlers
            |l| l.clear_handlers(),            // reset existing handlers
            |l, h| l.add_handler(h),           // attach handler to logger
            Self::duplicate_handler_ids_error, // error builder for duplicates
        )?;
        self.apply_items(
            &logger_ref,                      // logger to mutate
            cfg.filter_ids(),                 // declared filter identifiers
            filters,                          // pool of built filters
            |l| l.clear_filters(),            // reset existing filters
            |l, f| l.add_filter(f),           // attach filter to logger
            Self::duplicate_filter_ids_error, // error builder for duplicates
        )?;
        if let Some(level) = cfg.level_opt() {
            logger_ref.set_level(level);
        }
        Ok(())
    }

    fn duplicate_handler_ids_error(ids: Vec<String>) -> ConfigError {
        ConfigError::DuplicateHandlerIds(ids)
    }

    fn duplicate_filter_ids_error(ids: Vec<String>) -> ConfigError {
        ConfigError::DuplicateFilterIds(ids)
    }

    /// Apply a sequence of items to `logger_ref`.
    ///
    /// * `logger_ref` - logger being mutated.
    /// * `ids` - ordered identifiers to resolve.
    /// * `pool` - mapping of identifiers to built items.
    /// * `clear` - clears existing items before attachment.
    /// * `add` - attaches a resolved item to the logger.
    /// * `dup_err` - constructs a duplicate identifier error.
    fn apply_items<T: ?Sized>(
        &self,
        logger_ref: &PyRef<FemtoLogger>,
        ids: &[String],
        pool: &BTreeMap<String, Arc<T>>,
        clear: impl Fn(&PyRef<FemtoLogger>),
        add: impl Fn(&PyRef<FemtoLogger>, Arc<T>),
        dup_err: impl Fn(Vec<String>) -> ConfigError,
    ) -> Result<(), ConfigError> {
        let mut seen = HashSet::new();
        let mut dup = Vec::new();
        let mut items = Vec::new();
        for id in ids {
            if !seen.insert(id) {
                dup.push(id.clone());
                continue;
            }
            let item = pool
                .get(id)
                .cloned()
                .ok_or_else(|| ConfigError::UnknownId(id.clone()))?;
            items.push(item);
        }
        if !dup.is_empty() {
            return Err(dup_err(dup));
        }
        clear(logger_ref);
        for item in items {
            add(logger_ref, item);
        }
        Ok(())
    }
}
