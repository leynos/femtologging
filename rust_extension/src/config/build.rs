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

macro_rules! apply_items {
    (
        $logger_ref:expr,
        $ids:expr,
        $pool:expr,
        $clear:ident,
        $add:ident,
        $dup_err:ident
    ) => {{
        let mut seen = HashSet::new();
        let mut dup = Vec::new();
        let mut items = Vec::new();

        for id in $ids {
            if !seen.insert(id) {
                dup.push(id.clone());
                continue;
            }
            let item = $pool
                .get(id)
                .cloned()
                .ok_or_else(|| ConfigError::UnknownId(id.clone()))?;
            items.push(item);
        }
        if !dup.is_empty() {
            return Err(ConfigError::$dup_err(dup));
        }
        $logger_ref.$clear();
        for item in items {
            $logger_ref.$add(item);
        }
        Ok(())
    }};
}

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
        apply_items!(
            logger_ref,          // logger to mutate
            cfg.handler_ids(),   // declared handler identifiers
            handlers,            // pool of built handlers
            clear_handlers,      // reset existing handlers
            add_handler,         // attach handler to logger
            DuplicateHandlerIds  // error builder for duplicates
        )?;
        apply_items!(
            logger_ref,         // logger to mutate
            cfg.filter_ids(),   // declared filter identifiers
            filters,            // pool of built filters
            clear_filters,      // reset existing filters
            add_filter,         // attach filter to logger
            DuplicateFilterIds  // error builder for duplicates
        )?;
        if let Some(level) = cfg.level_opt() {
            logger_ref.set_level(level);
        }
        Ok(())
    }
}
