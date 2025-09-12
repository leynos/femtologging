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
///
/// use crate::config::build::logger_names_with_ancestors;
/// use crate::config::LoggerConfigBuilder;
/// use std::collections::BTreeMap;
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
    /// Collect items for later application to a logger.
    ///
    /// Preallocates internal buffers to avoid reallocations.
    ///
    /// Deduplicates `ids`, raising `dup_err` if any repeats are found. Each
    /// duplicate identifier is reported once. The matched objects are
    /// returned so callers can clear any existing logger state and attach
    /// the items.
    fn collect_items<T: ?Sized>(
        ids: &[String],
        pool: &BTreeMap<String, Arc<T>>,
        dup_err: impl FnOnce(Vec<String>) -> ConfigError,
    ) -> Result<Vec<Arc<T>>, ConfigError> {
        // 1) Detect duplicates first (report each duplicate once, preserve
        // first-seen order) so we fail deterministically before any pool
        // lookups. This keeps error behaviour stable regardless of map
        // contents or lookup costs.
        let mut seen = HashSet::with_capacity(ids.len());
        let mut dup_once: HashSet<&str> = HashSet::new();
        let mut dup = Vec::new();
        for id in ids {
            if !seen.insert(id) && dup_once.insert(id.as_str()) {
                dup.push(id.clone());
            }
        }
        if !dup.is_empty() {
            return Err(dup_err(dup));
        }

        // 2) Perform lookups only after confirming no duplicates exist.
        let mut items = Vec::with_capacity(ids.len());
        for id in ids {
            let obj = pool
                .get(id)
                .cloned()
                .ok_or_else(|| ConfigError::UnknownId(id.clone()))?;
            items.push(obj);
        }
        Ok(items)
    }

    /// Generic helper for both handlers and filters
    ///
    /// Uses `collect_items` in this module to preserve improved duplicate
    /// reporting and allocation behaviour, then applies the items via the
    /// provided closures to avoid duplication across handlers/filters. When
    /// both duplicate IDs and unknown IDs are present, duplicate-ID errors
    /// take precedence and return before unknown-ID errors.
    fn apply_items<T: ?Sized>(
        &self,
        logger_ref: &PyRef<FemtoLogger>,
        ids: &[String],
        pool: &BTreeMap<String, Arc<T>>,
        clear_fn: impl FnOnce(&PyRef<FemtoLogger>),
        add_fn: impl Fn(&PyRef<FemtoLogger>, Arc<T>),
        dup_err: impl FnOnce(Vec<String>) -> ConfigError,
    ) -> Result<(), ConfigError> {
        let items = Self::collect_items(ids, pool, dup_err)?;
        clear_fn(logger_ref);
        for item in items {
            add_fn(logger_ref, item);
        }
        Ok(())
    }
    fn apply_handlers(
        &self,
        logger_ref: &PyRef<FemtoLogger>,
        ids: &[String],
        pool: &BTreeMap<String, Arc<dyn FemtoHandlerTrait>>,
    ) -> Result<(), ConfigError> {
        self.apply_items(
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
        self.apply_items(
            logger_ref,
            ids,
            pool,
            |l| l.clear_filters(),
            |l, f| l.add_filter(f),
            ConfigError::DuplicateFilterIds,
        )
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
