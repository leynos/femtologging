//! Construction and realisation of configuration.
#![cfg(feature = "python")]

use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use pyo3::prelude::*;

use crate::config::ConfigError;
use crate::{filters::FemtoFilter, handler::FemtoHandlerTrait, logger::FemtoLogger, manager};

use super::types::{ConfigBuilder, LoggerConfigBuilder};
<<<<<<< HEAD
||||||| parent of 73c14d9 (Generalise apply_items helper)

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
        let mut missing = Vec::new();
        let mut items = Vec::new();

        for id in $ids {
            if !seen.insert(id) {
                dup.push(id.clone());
                continue;
            }
            match $pool.get(id).cloned() {
                Some(item) => items.push(item),
                None => missing.push(id.clone()),
            }
        }
        if !dup.is_empty() {
            return Err(ConfigError::$dup_err(dup));
        }
        if !missing.is_empty() {
            return Err(ConfigError::UnknownIds(missing));
        }
        $logger_ref.$clear();
        for item in items {
            $logger_ref.$add(item);
        }
        Ok(())
    }};
}

=======

>>>>>>> 73c14d9 (Generalise apply_items helper)
impl ConfigBuilder {
    /// Finalise the configuration and initialise loggers.
    pub fn build_and_init(&self) -> Result<(), ConfigError> {
        if self.version() != 1 {
            return Err(ConfigError::UnsupportedVersion(self.version()));
        }
        if self.root_logger().is_none() {
            return Err(ConfigError::MissingRootLogger);
        }
        let built_handlers = Self::build_map(
            self.handler_builders(),
            |b| b.build(),
            |id, source| ConfigError::HandlerBuild { id, source },
        )?;
        let built_filters = Self::build_map(
            self.filter_builders(),
            |b| b.build(),
            |id, source| ConfigError::FilterBuild { id, source },
        )?;

        Python::with_gil(|py| -> Result<_, ConfigError> {
            // Handle disable_existing_loggers if requested
            if self.disable_existing_loggers() {
                let mut keep_names: HashSet<String> = self
                    .logger_builders()
                    .keys()
                    .cloned()
                    .chain(std::iter::once("root".to_string()))
                    .collect();
                // Include ancestors of each kept logger (e.g., "a.b.c" keeps "a.b" and "a").
                for name in self.logger_builders().keys() {
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
                .root_logger()
                .map(|c| ("root", c))
                .into_iter()
                .chain(self.logger_builders().iter().map(|(n, c)| (n.as_str(), c)));

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

<<<<<<< HEAD
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

    // Apply a sequence of items to a logger, clearing existing state first.
    // - `clear` removes existing handlers or filters.
    // - `add` attaches each collected item.
    fn apply_items<T: ?Sized>(
        &self,
        logger_ref: &PyRef<FemtoLogger>,
        ids: &[String],
        pool: &BTreeMap<String, Arc<T>>,
        clear: impl Fn(&PyRef<FemtoLogger>),
        add: impl Fn(&PyRef<FemtoLogger>, Arc<T>),
        dup_err: impl Fn(Vec<String>) -> ConfigError,
    ) -> Result<(), ConfigError> {
        let items = Self::collect_items(ids, pool, dup_err)?;
        clear(logger_ref);
        for item in items {
            add(logger_ref, item);
        }
        Ok(())
    }

    fn duplicate_handler_ids(ids: Vec<String>) -> ConfigError {
        ConfigError::DuplicateHandlerIds(ids)
    }

    fn duplicate_filter_ids(ids: Vec<String>) -> ConfigError {
        ConfigError::DuplicateFilterIds(ids)
||||||| parent of 73c14d9 (Generalise apply_items helper)
=======
    fn apply_items<T: ?Sized, I, S>(
        &self,
        logger_ref: &PyRef<FemtoLogger>,
        ids: I,
        pool: &BTreeMap<String, Arc<T>>,
        clear: impl Fn(&PyRef<FemtoLogger>),
        add: impl Fn(&PyRef<FemtoLogger>, &Arc<T>),
        dup_err: impl Fn(Vec<String>) -> ConfigError,
    ) -> Result<(), ConfigError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut seen = HashSet::new();
        let mut dup = Vec::new();
        let mut missing = Vec::new();
        let mut items = Vec::new();

        for id in ids {
            let id_ref = id.as_ref();
            if !seen.insert(id_ref.to_owned()) {
                dup.push(id_ref.to_owned());
                continue;
            }
            match pool.get(id_ref).cloned() {
                Some(item) => items.push(item),
                None => missing.push(id_ref.to_owned()),
            }
        }
        if !dup.is_empty() {
            return Err(dup_err(dup));
        }
        if !missing.is_empty() {
            return Err(ConfigError::UnknownIds(missing));
        }
        clear(logger_ref);
        for item in &items {
            add(logger_ref, item);
        }
        Ok(())
>>>>>>> 73c14d9 (Generalise apply_items helper)
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
<<<<<<< HEAD
            &logger_ref,
            cfg.handler_ids(),
            handlers,
            |l| l.clear_handlers(),
            |l, h| l.add_handler(h),
            Self::duplicate_handler_ids,
||||||| parent of 73c14d9 (Generalise apply_items helper)
        apply_items!(
            logger_ref,          // logger to mutate
            cfg.handler_ids(),   // declared handler identifiers
            handlers,            // pool of built handlers
            clear_handlers,      // reset existing handlers
            add_handler,         // attach handler to logger
            DuplicateHandlerIds  // error builder for duplicates
=======
            &logger_ref,                         // logger to mutate
            cfg.handler_ids(),                   // declared handler identifiers
            handlers,                            // pool of built handlers
            |l| l.clear_handlers(),              // reset existing handlers
            |l, h| l.add_handler(Arc::clone(h)), // attach handler to logger
            ConfigError::DuplicateHandlerIds,    // error builder for duplicates
>>>>>>> 73c14d9 (Generalise apply_items helper)
        )?;
        self.apply_items(
<<<<<<< HEAD
            &logger_ref,
            cfg.filter_ids(),
            filters,
            |l| l.clear_filters(),
            |l, f| l.add_filter(f),
            Self::duplicate_filter_ids,
||||||| parent of 73c14d9 (Generalise apply_items helper)
        apply_items!(
            logger_ref,         // logger to mutate
            cfg.filter_ids(),   // declared filter identifiers
            filters,            // pool of built filters
            clear_filters,      // reset existing filters
            add_filter,         // attach filter to logger
            DuplicateFilterIds  // error builder for duplicates
=======
            &logger_ref,                        // logger to mutate
            cfg.filter_ids(),                   // declared filter identifiers
            filters,                            // pool of built filters
            |l| l.clear_filters(),              // reset existing filters
            |l, f| l.add_filter(Arc::clone(f)), // attach filter to logger
            ConfigError::DuplicateFilterIds,    // error builder for duplicates
>>>>>>> 73c14d9 (Generalise apply_items helper)
        )?;
        if let Some(level) = cfg.level_opt().or(self.default_level()) {
            logger_ref.set_level(level);
        }
        if let Some(propagate) = cfg.propagate_opt() {
            logger_ref.set_propagate(propagate);
        }
        Ok(())
    }
}
