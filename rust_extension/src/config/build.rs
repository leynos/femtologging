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
    fn apply_collection<T: ?Sized, FClear, FAdd, FDup>(
        &self,
        logger_ref: &PyRef<FemtoLogger>,
        ids: &[String],
        pool: &BTreeMap<String, Arc<T>>,
        clear: FClear,
        add: FAdd,
        dup_err: FDup,
    ) -> Result<(), ConfigError>
    where
        FClear: Fn(&PyRef<FemtoLogger>),
        FAdd: Fn(&PyRef<FemtoLogger>, Arc<T>),
        FDup: Fn(Vec<String>) -> ConfigError,
    {
        let mut seen = HashSet::new();
        let mut dup = Vec::new();
        let mut items = Vec::new();
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
        clear(logger_ref);
        for item in items {
            add(logger_ref, item);
        }
        Ok(())
    }

    fn apply_handlers(
        &self,
        logger_ref: &PyRef<FemtoLogger>,
        ids: &[String],
        pool: &BTreeMap<String, Arc<dyn FemtoHandlerTrait>>,
    ) -> Result<(), ConfigError> {
        self.apply_collection(
            logger_ref,
            ids,
            pool,
            |l| l.clear_handlers(),
            |l, h| l.add_handler(h),
            ConfigError::DuplicateHandlerIds,
        )
    }

    fn apply_filters(
        &self,
        logger_ref: &PyRef<FemtoLogger>,
        ids: &[String],
        pool: &BTreeMap<String, Arc<dyn FemtoFilter>>,
    ) -> Result<(), ConfigError> {
        self.apply_collection(
            logger_ref,
            ids,
            pool,
            |l| l.clear_filters(),
            |l, f| l.add_filter(f),
            ConfigError::DuplicateFilterIds,
        )
    }
}
