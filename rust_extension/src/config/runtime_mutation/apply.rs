//! Applies runtime configuration mutations to the live logger state.

use std::collections::{BTreeMap, BTreeSet};

use pyo3::prelude::*;

use crate::{
    config::reachability::{
        ReachableIds, unreachable_definition_error, unreachable_definition_ids,
    },
    config::types::HandlerBuilder,
    filters::FilterBuilder,
    manager::{LoggerAttachmentState, RuntimeStateSnapshot},
};

use super::{
    BuiltRegistries, ConfigError, LoggerMutationBuilder, LoggerScalarMutation,
    RuntimeConfigBuilder, apply_commit, apply_mutation_to_logger, build_filters, build_handlers,
};

impl RuntimeConfigBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_handler<B>(mut self, id: impl Into<String>, builder: B) -> Self
    where
        B: Into<HandlerBuilder>,
    {
        self.handlers.insert(id.into(), builder.into());
        self
    }

    pub fn with_filter(mut self, id: impl Into<String>, builder: FilterBuilder) -> Self {
        self.filters.insert(id.into(), builder);
        self
    }

    pub fn with_logger(mut self, name: impl Into<String>, builder: LoggerMutationBuilder) -> Self {
        self.loggers.insert(name.into(), builder);
        self
    }

    pub fn with_root_logger(mut self, builder: LoggerMutationBuilder) -> Self {
        self.root_logger = Some(builder);
        self
    }

    /// Apply the runtime mutation transactionally.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::InvalidMutation`] when a supplied handler or
    /// filter is unreachable from the resulting logger graph, when collection
    /// operations conflict, or when a mutation requires runtime metadata that
    /// is unavailable. Returns [`ConfigError::UnknownIds`] when an attachment
    /// or handler-level filter references an identifier absent from both the
    /// manager and this transaction.
    ///
    /// Unreachable definitions are rejected before construction, so a rejected
    /// transaction never opens a file, connects a socket, or spawns a worker.
    pub fn apply(&self) -> Result<(), ConfigError> {
        self.validate()?;
        Python::attach(|py| -> Result<(), ConfigError> {
            let _transaction = super::super::lock_runtime_transaction();
            let before = crate::manager::snapshot_runtime_state();
            let plan = self.plan_mutation(&before)?;
            self.validate_handler_filter_ids(&before)?;
            let handler_filter_ids = self.planned_handler_filter_ids(&before);
            self.reject_unreachable_definitions(&plan, &handler_filter_ids)?;

            let built = BuiltRegistries {
                handlers: build_handlers(&self.handlers)?,
                filters: build_filters(&self.filters)?,
                handler_filter_ids: self
                    .handlers
                    .iter()
                    .map(|(id, builder)| (id.clone(), builder.filter_ids().to_vec()))
                    .collect(),
            };
            let commit = self.assemble_commit(py, before, plan, built)?;
            apply_commit(py, &commit)?;
            Ok(())
        })
    }

    /// Validate handler-level filter references before any handler is built.
    fn validate_handler_filter_ids(
        &self,
        before: &RuntimeStateSnapshot,
    ) -> Result<(), ConfigError> {
        for builder in self.handlers.values() {
            for id in builder.filter_ids() {
                if !before.filter_registry.contains_key(id) && !self.filters.contains_key(id) {
                    return Err(ConfigError::UnknownIds(vec![id.clone()]));
                }
            }
        }
        Ok(())
    }

    /// Merge current handler filter references with definitions in this transaction.
    fn planned_handler_filter_ids(
        &self,
        before: &RuntimeStateSnapshot,
    ) -> BTreeMap<String, Vec<String>> {
        let mut handler_filter_ids = before.handler_filter_ids.clone();
        handler_filter_ids.extend(
            self.handlers
                .iter()
                .map(|(id, builder)| (id.clone(), builder.filter_ids().to_vec())),
        );
        handler_filter_ids
    }

    /// Reject definitions that the resulting logger and handler graph cannot reach.
    fn reject_unreachable_definitions(
        &self,
        plan: &super::PlannedMutation,
        handler_filter_ids: &BTreeMap<String, Vec<String>>,
    ) -> Result<(), ConfigError> {
        let reachable = ReachableIds::from_logger_states_and_handler_filters(
            plan.logger_states.values(),
            handler_filter_ids,
        );
        let supplied = ReachableIds::from_definition_ids(self.handlers.keys(), self.filters.keys());
        match unreachable_definition_ids(&supplied, &reachable) {
            Some((handlers, filters)) => Err(unreachable_definition_error(&handlers, &filters)),
            None => Ok(()),
        }
    }

    /// Rejects duplicate root addressing and any previously recorded collection conflict.
    fn validate(&self) -> Result<(), ConfigError> {
        if self.root_logger.is_some() && self.loggers.contains_key("root") {
            return Err(ConfigError::InvalidMutation(
                "root logger cannot be mutated via both with_root_logger() and with_logger(\"root\", ...)"
                    .to_string(),
            ));
        }
        if let Some(root) = &self.root_logger {
            root.ensure_valid("root")?;
        }
        for (name, mutation) in &self.loggers {
            mutation.ensure_valid(name)?;
        }
        Ok(())
    }

    /// Finds loggers affected by new registry entries or explicit logger mutations.
    pub(super) fn collect_impacted(&self, before: &RuntimeStateSnapshot) -> BTreeSet<String> {
        let overridden_handler_ids = self.handlers.keys().cloned().collect::<BTreeSet<_>>();
        let overridden_filter_ids = self.filters.keys().cloned().collect::<BTreeSet<_>>();
        let mut impacted = before
            .logger_states
            .iter()
            .filter(|(_, state)| {
                state
                    .handler_ids()
                    .iter()
                    .any(|id| overridden_handler_ids.contains(id))
                    || state
                        .filter_ids()
                        .iter()
                        .any(|id| overridden_filter_ids.contains(id))
                    || state.handler_ids().iter().any(|handler_id| {
                        before
                            .handler_filter_ids
                            .get(handler_id)
                            .is_some_and(|filter_ids| {
                                filter_ids
                                    .iter()
                                    .any(|id| overridden_filter_ids.contains(id))
                            })
                    })
            })
            .map(|(name, _)| name.clone())
            .collect::<BTreeSet<_>>();
        if self.root_logger.is_some() {
            impacted.insert("root".to_string());
        }
        impacted.extend(self.loggers.keys().cloned());
        impacted
    }

    /// Applies root and named collection mutations to a private state map before publication.
    pub(super) fn apply_logger_mutations(
        &self,
        logger_states: &mut BTreeMap<String, LoggerAttachmentState>,
        known: &super::KnownIds,
    ) -> Result<(), ConfigError> {
        let root_iter = self.root_logger.iter().map(|m| ("root", m));
        let named_iter = self.loggers.iter().map(|(n, m)| (n.as_str(), m));
        for (name, mutation) in root_iter.chain(named_iter) {
            apply_mutation_to_logger(name, mutation, logger_states, known)?;
        }
        Ok(())
    }

    /// Extracts scalar overrides for the logger names addressed by this request.
    pub(super) fn build_scalar_mutations(&self) -> BTreeMap<String, LoggerScalarMutation> {
        let mut out = BTreeMap::new();
        if let Some(root) = &self.root_logger {
            out.insert(
                "root".to_string(),
                LoggerScalarMutation {
                    level: root.level,
                    propagate: root.propagate,
                },
            );
        }
        for (name, mutation) in &self.loggers {
            out.insert(
                name.clone(),
                LoggerScalarMutation {
                    level: mutation.level,
                    propagate: mutation.propagate,
                },
            );
        }
        out
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for runtime mutation planning.

    use rstest::rstest;

    use super::{RuntimeConfigBuilder, RuntimeStateSnapshot};

    /// Confirms an empty request leaves the impacted logger set empty.
    #[rstest]
    fn empty_mutation_has_no_impacted_loggers() {
        let impacted =
            RuntimeConfigBuilder::new().collect_impacted(&RuntimeStateSnapshot::default());

        assert!(
            impacted.is_empty(),
            "an empty mutation must not fetch or rewrite any logger",
        );
    }
}
