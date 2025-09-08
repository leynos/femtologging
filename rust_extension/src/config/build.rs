//! Construction and realisation of configuration.

use std::{collections::BTreeMap, sync::Arc};

use pyo3::prelude::*;

use crate::{filters::FemtoFilter, handler::FemtoHandlerTrait, logger::FemtoLogger, manager};

use super::types::{ConfigBuilder, ConfigError, LoggerConfigBuilder};

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

        Python::with_gil(|py| -> Result<(), ConfigError> {
            let mut targets: Vec<(&str, &LoggerConfigBuilder)> = Vec::new();
            if let Some(root_cfg) = &self.root_logger {
                targets.push(("root", root_cfg));
            }
            targets.extend(self.loggers.iter().map(|(n, c)| (n.as_str(), c)));

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
        if let Some(level) = cfg.level_opt() {
            logger.borrow(py).set_level(level);
        }
        for hid in cfg.handler_ids() {
            let h = handlers
                .get(hid)
                .ok_or_else(|| ConfigError::UnknownHandlerId(hid.clone()))?;
            logger.borrow(py).add_handler(h.clone());
        }
        let resolved_filters: Vec<_> = cfg
            .filter_ids()
            .iter()
            .map(|fid| {
                filters
                    .get(fid)
                    .cloned()
                    .ok_or_else(|| ConfigError::UnknownFilterId(fid.clone()))
            })
            .collect::<Result<_, _>>()?;
        // Replace existing filters only after all filter IDs validate.
        {
            let logger_ref = logger.borrow(py);
            logger_ref.clear_filters();
            for f in resolved_filters {
                logger_ref.add_filter(f);
            }
        }

        Ok(())
    }
}
