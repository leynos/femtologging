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

/// Collect logger names and their ancestors for disable_existing_loggers.
///
/// When disabling existing loggers, direct ancestors must also be kept so
/// they are not inadvertently removed.
///
/// # Examples
/// ```ignore
/// use std::collections::BTreeMap;
/// use crate::config::build::logger_names_with_ancestors;
/// use crate::config::LoggerConfigBuilder;
/// let mut loggers = BTreeMap::new();
/// loggers.insert("a.b.c".to_string(), LoggerConfigBuilder::new());
/// let names = logger_names_with_ancestors(&loggers);
/// assert!(names.contains("a.b"));
/// assert!(names.contains("a"));
/// assert!(names.contains("root"));
/// ```
pub(crate) fn logger_names_with_ancestors(
    loggers: &BTreeMap<String, LoggerConfigBuilder>,
) -> HashSet<String> {
    // Pre-size to minimise reallocations: keys + approx ancestors ("." count) + "root".
    let approx_ancestors = loggers
        .keys()
        .map(|k| k.matches('.').count())
        .sum::<usize>();
    let mut keep_names: HashSet<String> =
        HashSet::with_capacity(loggers.len() + approx_ancestors + 1);
    keep_names.extend(
        loggers
            .keys()
            .cloned()
            .chain(std::iter::once("root".to_string())),
    );
    for name in loggers.keys() {
        let mut cur = name.as_str();
        while let Some((parent, _)) = cur.rsplit_once('.') {
            keep_names.insert(parent.to_string());
            cur = parent;
        }
    }
    keep_names
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
                let keep_names = logger_names_with_ancestors(&self.loggers);
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

    fn apply_handlers(
        &self,
        logger_ref: &PyRef<FemtoLogger>,
        ids: &[String],
        pool: &BTreeMap<String, Arc<dyn FemtoHandlerTrait>>,
    ) -> Result<(), ConfigError> {
        let mut seen = HashSet::new();
        let mut dup = Vec::new();
        let mut items = Vec::new();
        for id in ids {
            if !seen.insert(id) {
                dup.push(id.clone());
                continue;
            }
            let handler = pool
                .get(id)
                .cloned()
                .ok_or_else(|| ConfigError::UnknownId(id.clone()))?;
            items.push(handler);
        }
        if !dup.is_empty() {
            return Err(ConfigError::DuplicateHandlerIds(dup));
        }
        logger_ref.clear_handlers();
        for handler in items {
            logger_ref.add_handler(handler);
        }
        Ok(())
    }

    fn apply_filters(
        &self,
        logger_ref: &PyRef<FemtoLogger>,
        ids: &[String],
        pool: &BTreeMap<String, Arc<dyn FemtoFilter>>,
    ) -> Result<(), ConfigError> {
        let mut seen = HashSet::new();
        let mut dup = Vec::new();
        let mut items = Vec::new();
        for id in ids {
            if !seen.insert(id) {
                dup.push(id.clone());
                continue;
            }
            let filter = pool
                .get(id)
                .cloned()
                .ok_or_else(|| ConfigError::UnknownId(id.clone()))?;
            items.push(filter);
        }
        if !dup.is_empty() {
            return Err(ConfigError::DuplicateFilterIds(dup));
        }
        logger_ref.clear_filters();
        for filter in items {
            logger_ref.add_filter(filter);
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    //! Tests for logger ancestor collection helper.

    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case(
        {
            let mut loggers = BTreeMap::new();
            loggers.insert("a.b.c".to_string(), LoggerConfigBuilder::new());
            loggers.insert("top".to_string(), LoggerConfigBuilder::new());
            loggers
        },
        ["a.b.c", "a.b", "a", "top", "root"]
            .into_iter()
            .map(String::from)
            .collect::<HashSet<_>>()
    )]
    #[case(
        BTreeMap::new(),
        ["root"].into_iter().map(String::from).collect::<HashSet<_>>()
    )]
    #[case(
        {
            let mut loggers = BTreeMap::new();
            loggers.insert("alpha".to_string(), LoggerConfigBuilder::new());
            loggers.insert("beta".to_string(), LoggerConfigBuilder::new());
            loggers
        },
        ["alpha", "beta", "root"]
            .into_iter()
            .map(String::from)
            .collect::<HashSet<_>>()
    )]
    #[case(
        {
            let mut loggers = BTreeMap::new();
            loggers.insert("a..b...c".to_string(), LoggerConfigBuilder::new());
            loggers
        },
        [
            "a..b...c",
            "a..b..",
            "a..b.",
            "a..b",
            "a.",
            "a",
            "root"
        ]
        .into_iter()
        .map(String::from)
        .collect::<HashSet<_>>()
    )]
    fn logger_names_with_ancestors_returns_expected(
        #[case] loggers: BTreeMap<String, LoggerConfigBuilder>,
        #[case] expected: HashSet<String>,
    ) {
        let names = logger_names_with_ancestors(&loggers);
        assert_eq!(names, expected);
    }
}
