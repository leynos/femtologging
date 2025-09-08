#![cfg(feature = "python")]
//! Construction and realisation of configuration.

use std::{collections::BTreeMap, sync::Arc};

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
        manager::get_logger(py, name).map_err(|e| ConfigError::LoggerInit(e.to_string()))
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

        if let Some(level) = cfg.level_opt() {
            logger_ref.set_level(level);
        }

        Self::apply_handlers(&logger_ref, cfg.handler_ids(), handlers)?;
        Self::apply_filters(&logger_ref, cfg.filter_ids(), filters)?;

        Ok(())
    }

    fn collect_items<T: ?Sized, F>(
        ids: &[String],
        pool: &BTreeMap<String, Arc<T>>,
        mk_err: F,
    ) -> Result<Vec<Arc<T>>, ConfigError>
    where
        F: Fn(String) -> ConfigError,
    {
        ids.iter()
            .map(|id| pool.get(id).cloned().ok_or_else(|| mk_err(id.clone())))
            .collect()
    }

    fn apply_handlers(
        logger_ref: &PyRef<FemtoLogger>,
        ids: &[String],
        handlers: &BTreeMap<String, Arc<dyn FemtoHandlerTrait>>,
    ) -> Result<(), ConfigError> {
        let items = Self::collect_items(ids, handlers, ConfigError::UnknownHandlerId)?;
        items.into_iter().for_each(|h| logger_ref.add_handler(h));
        Ok(())
    }

    fn apply_filters(
        logger_ref: &PyRef<FemtoLogger>,
        ids: &[String],
        filters: &BTreeMap<String, Arc<dyn FemtoFilter>>,
    ) -> Result<(), ConfigError> {
        let items = Self::collect_items(ids, filters, ConfigError::UnknownFilterId)?;
        logger_ref.clear_filters();
        items.into_iter().for_each(|f| logger_ref.add_filter(f));
        Ok(())
    }
}
