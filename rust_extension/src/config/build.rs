//! Construction and realisation of configuration.
//!
//! `ConfigBuilder::build_and_init` first validates and resolves a complete
//! configuration without changing live loggers. It then asks the manager to
//! apply that prepared state in one infallible commit.

use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use pyo3::prelude::*;

use crate::config::ConfigError;
use crate::{
    filters::FemtoFilter,
    handler::FemtoHandlerTrait,
    level::FemtoLevel,
    logger::HandlerAttachment,
    manager,
};

use super::types::{ConfigBuilder, LoggerConfigBuilder};

/// A logger plan whose resource references have been fully resolved.
struct ConfiguredLoggerPlan {
    /// Registry name this plan will configure.
    name: String,
    /// Handler identities retained for later runtime updates.
    handler_ids: Vec<String>,
    /// Filter identities retained for later runtime updates.
    filter_ids: Vec<String>,
    /// Handler attachments with their producer-side filter chains.
    handlers: Vec<HandlerAttachment>,
    /// Logger-level filters applied before dispatch.
    filters: Vec<Arc<dyn FemtoFilter>>,
    /// Optional threshold that leaves the current level intact when absent.
    level: Option<FemtoLevel>,
    /// Optional propagation choice that leaves the current value intact.
    propagate: Option<bool>,
}

/// Runtime registries and handler filter chains prepared before commit.
struct BuiltConfigComponents {
    /// Handler instances keyed by configuration identifiers.
    built_handlers: BTreeMap<String, Arc<dyn FemtoHandlerTrait>>,
    /// Handler filter IDs retained for later runtime updates.
    handler_filter_ids: BTreeMap<String, Vec<String>>,
    /// Filter instances keyed by configuration identifiers.
    built_filters: BTreeMap<String, Arc<dyn FemtoFilter>>,
    /// Resolved filter chains keyed by handler ID, in configured order.
    resolved_handler_filters: BTreeMap<String, Vec<Arc<dyn FemtoFilter>>>,
}

impl ConfigBuilder {
    /// Validate, prepare and apply this complete logger configuration.
    ///
    /// The transition is atomic from the caller's perspective: either every
    /// logger receives the prepared state, or a rejected configuration leaves
    /// all pre-existing logger state unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] for an unsupported version, missing root,
    /// malformed logger name, unknown or duplicate resource ID, failed
    /// handler/filter construction, or incompatible Python handler dispatch.
    /// Every such error is found before live logger state is changed.
    pub fn build_and_init(&self) -> Result<(), ConfigError> {
        if self.version() != 1 {
            return Err(ConfigError::UnsupportedVersion(self.version()));
        }
        if self.root_logger().is_none() {
            return Err(ConfigError::MissingRootLogger);
        }

        // Reject malformed names before handler construction can open files.
        for name in self.configured_logger_names() {
            manager::validate_logger_name(&name)
                .map_err(|error| ConfigError::LoggerInit(error.to_string()))?;
        }

        let components = self.build_runtime_components()?;
        let displaced = Python::attach(move |py| -> Result<_, ConfigError> {
            let targets = self
                .root_logger()
                .map(|config| ("root", config))
                .into_iter()
                .chain(self.logger_builders().iter().map(|(name, config)| {
                    (name.as_str(), config)
                }));
            let plans = targets
                .map(|(name, config)| self.prepare_logger_plan(name, config, &components))
                .collect::<Result<Vec<_>, _>>()?;
            let loggers = self.pair_plans_with_loggers(py, plans)?;

            Ok(manager::commit_configuration(
                py,
                manager::PreparedConfiguration {
                    loggers,
                    disable_existing: self.disable_existing_loggers(),
                    keep_names: self.build_keep_names(),
                    handler_registry: components.built_handlers,
                    handler_filter_ids: components.handler_filter_ids,
                    filter_registry: components.built_filters,
                },
            ))
        })?;

        // Handler teardown can flush or join a worker, so release all displaced
        // references after the manager lock and Python attachment have ended.
        let manager::DisplacedAttachments {
            handlers,
            filters,
            unused_loggers,
            runtime_state,
        } = displaced;
        drop((handlers, filters, unused_loggers, runtime_state));
        Ok(())
    }

    /// Names targeted by the configuration, with the root logger first.
    fn configured_logger_names(&self) -> Vec<String> {
        std::iter::once("root".to_string())
            .chain(self.logger_builders().keys().cloned())
            .collect()
    }

    /// Pair each resolved plan with an existing logger or an unregistered one.
    ///
    /// This reads the registry without changing it. If another caller
    /// registers a name before commit, the manager adopts that live logger.
    fn pair_plans_with_loggers(
        &self,
        py: Python<'_>,
        plans: Vec<ConfiguredLoggerPlan>,
    ) -> Result<Vec<manager::PreparedLogger>, ConfigError> {
        let staged = manager::stage_loggers(py, plans.iter().map(|plan| plan.name.as_str()))
            .map_err(|error| ConfigError::LoggerInit(error.to_string()))?;
        plans
            .into_iter()
            .map(|plan| {
                let logger = staged.get(&plan.name).ok_or_else(|| {
                    ConfigError::LoggerInit(format!(
                        "missing staged logger for configuration plan {}",
                        plan.name
                    ))
                })?;
                Ok(manager::PreparedLogger {
                    name: plan.name,
                    logger: logger.clone_ref(py),
                    handlers: plan.handlers,
                    filters: plan.filters,
                    handler_ids: plan.handler_ids,
                    filter_ids: plan.filter_ids,
                    level: plan.level,
                    propagate: plan.propagate,
                })
            })
            .collect()
    }

    /// Compute the explicit logger names and ancestors protected from disabling.
    fn build_keep_names(&self) -> HashSet<String> {
        if !self.disable_existing_loggers() {
            return HashSet::new();
        }
        let mut names: HashSet<String> = self
            .logger_builders()
            .keys()
            .cloned()
            .chain(std::iter::once("root".to_string()))
            .collect();
        self.extend_keep_names_with_ancestors(&mut names);
        names
    }

    /// Include every dotted parent so descendants can continue to propagate.
    fn extend_keep_names_with_ancestors(&self, keep_names: &mut HashSet<String>) {
        for name in self.logger_builders().keys() {
            let mut current = name.as_str();
            while let Some((parent, _)) = current.rsplit_once('.') {
                keep_names.insert(parent.to_string());
                current = parent;
            }
        }
    }

    /// Build a named registry while preserving the failing configuration ID.
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
            let object = build(builder).map_err(|error| wrap_err(id.clone(), error))?;
            built.insert(id.clone(), object);
        }
        Ok(built)
    }

    /// Resolve resource references once and reject duplicate or unknown IDs.
    fn collect_items<T: ?Sized>(
        ids: &[String],
        pool: &BTreeMap<String, Arc<T>>,
        duplicate_error: impl FnOnce(Vec<String>) -> ConfigError,
    ) -> Result<Vec<Arc<T>>, ConfigError> {
        let mut seen = HashSet::new();
        let mut duplicates = Vec::new();
        let mut missing = Vec::new();
        let mut items = Vec::new();

        for id in ids {
            if !seen.insert(id.clone()) {
                duplicates.push(id.clone());
                continue;
            }
            match pool.get(id) {
                Some(item) => items.push(item.clone()),
                None => missing.push(id.clone()),
            }
        }

        if !duplicates.is_empty() {
            return Err(duplicate_error(duplicates));
        }
        if !missing.is_empty() {
            return Err(ConfigError::UnknownIds(missing));
        }
        Ok(items)
    }

    /// Convert repeated handler references into a configuration error.
    fn duplicate_handler_ids(ids: Vec<String>) -> ConfigError {
        ConfigError::DuplicateHandlerIds(ids)
    }

    /// Convert repeated filter references into a configuration error.
    fn duplicate_filter_ids(ids: Vec<String>) -> ConfigError {
        ConfigError::DuplicateFilterIds(ids)
    }

    /// Resolve and validate a logger's resource references without live mutation.
    fn prepare_logger_plan(
        &self,
        name: &str,
        config: &LoggerConfigBuilder,
        components: &BuiltConfigComponents,
    ) -> Result<ConfiguredLoggerPlan, ConfigError> {
        let resolved_handlers = Self::collect_items(
            config.handler_ids(),
            &components.built_handlers,
            Self::duplicate_handler_ids,
        )?;
        #[cfg(feature = "python")]
        if resolved_handlers
            .iter()
            .any(|handler| handler.is_python_backed() && !handler.provides_context_dispatch())
        {
            return Err(ConfigError::LoggerInit(
                "Python-backed handlers must provide contextual dispatch".to_owned(),
            ));
        }

        let handlers = config
            .handler_ids()
            .iter()
            .zip(resolved_handlers)
            .map(|(id, handler)| {
                let filters = components
                    .resolved_handler_filters
                    .get(id)
                    .ok_or_else(|| ConfigError::UnknownIds(vec![id.clone()]))?;
                Ok(HandlerAttachment::with_filters(handler, filters.clone()))
            })
            .collect::<Result<Vec<_>, ConfigError>>()?;
        let filters = Self::collect_items(
            config.filter_ids(),
            &components.built_filters,
            Self::duplicate_filter_ids,
        )?;
        Ok(ConfiguredLoggerPlan {
            name: name.to_string(),
            handler_ids: config.handler_ids().to_vec(),
            filter_ids: config.filter_ids().to_vec(),
            handlers,
            filters,
            level: config.level_opt().or(self.default_level()),
            propagate: config.propagate_opt(),
        })
    }

    /// Build formatter, filter and handler registries before changing loggers.
    fn build_runtime_components(&self) -> Result<BuiltConfigComponents, ConfigError> {
        let built_formatters = self
            .formatter_builders()
            .iter()
            .map(|(id, builder)| (id.clone(), builder.build()))
            .collect::<BTreeMap<_, _>>();
        let built_filters = Self::build_map(
            self.filter_builders(),
            |builder| builder.build(),
            |id, source| ConfigError::FilterBuild { id, source },
        )?;
        // Resolve handler filters before constructors can open files or start workers.
        let handler_filters = self
            .handler_builders()
            .iter()
            .map(|(id, builder)| {
                Self::collect_items(
                    builder.filter_ids(),
                    &built_filters,
                    Self::duplicate_filter_ids,
                )
                .map(|filters| (id.clone(), filters))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let built_handlers = Self::build_map(
            self.handler_builders(),
            |builder| builder.build_with_formatters(&built_formatters),
            |id, source| ConfigError::HandlerBuild { id, source },
        )?;
        let handler_filter_ids = self
            .handler_builders()
            .iter()
            .map(|(id, builder)| (id.clone(), builder.filter_ids().to_vec()))
            .collect();

        Ok(BuiltConfigComponents {
            built_handlers,
            handler_filter_ids,
            built_filters,
            resolved_handler_filters: handler_filters,
        })
    }
}
