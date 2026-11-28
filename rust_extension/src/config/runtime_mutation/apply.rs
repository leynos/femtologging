//! Applies runtime configuration mutations to the live logger state.
//!
//! `RuntimeConfigBuilder`, declared in the parent `runtime_mutation` module,
//! collects handler, filter and logger mutations and then applies them in one
//! pass. Application has a defined order: the mutation is planned first, which
//! computes the impacted loggers and scalar fields; handler-level filter
//! identifiers are then validated and unreachable definitions rejected; only
//! after that is anything published. A rejected mutation therefore does not
//! publish its planned configuration. What it does not undo
//! is work already done to get there: a handler or filter built before a later
//! failure keeps its side effects, such as a created file, and a logger
//! registered while the commit is assembled stays registered even though the
//! transaction is rejected. `validate_handler_filter_ids`,
//! `planned_handler_filter_ids` and `reject_unreachable_definitions` keep each
//! check separate; `collect_impacted`, `apply_logger_mutations` and
//! `build_scalar_mutations` do the planning.

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
    BuiltRegistries, CommitInputs, ConfigError, LoggerMutationBuilder, LoggerScalarMutation,
    RuntimeConfigBuilder, apply_commit, apply_mutation_to_logger, build_filters, build_handlers,
};

impl RuntimeConfigBuilder {
    /// Start an empty transaction that changes nothing until a `with_*` method is called.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a handler definition under `id`, replacing any earlier definition with the same id
    /// in this transaction.
    ///
    /// `apply` constructs the handler after validation and before it publishes the transaction,
    /// so a later failure can leave construction side effects, such as a created file.
    #[must_use]
    pub fn with_handler<B>(mut self, id: impl Into<String>, builder: B) -> Self
    where
        B: Into<HandlerBuilder>,
    {
        self.handlers.insert(id.into(), builder.into());
        self
    }

    /// Add a filter definition under `id`, replacing any earlier definition with the same id
    /// in this transaction.
    ///
    /// Handlers and loggers refer to the filter by this id. `apply` builds the filter before it
    /// publishes the transaction, so a later failure can leave construction side effects.
    #[must_use]
    pub fn with_filter(mut self, id: impl Into<String>, builder: FilterBuilder) -> Self {
        self.filters.insert(id.into(), builder);
        self
    }

    /// Request changes to the logger called `name`, replacing any earlier request for that
    /// name in this transaction.
    ///
    /// Naming `"root"` here is rejected by `apply` when `with_root_logger` is also used. Use
    /// `with_root_logger` for the root logger.
    #[must_use]
    pub fn with_logger(mut self, name: impl Into<String>, builder: LoggerMutationBuilder) -> Self {
        self.loggers.insert(name.into(), builder);
        self
    }

    /// Request changes to the root logger, replacing any earlier root request in this
    /// transaction.
    #[must_use]
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
    /// Validation failures and unreachable definitions are rejected before any handler or
    /// filter is built, so those rejections never open a file, connect a socket, or spawn a
    /// worker. Errors from later steps, such as logger lookup or publication, can occur after
    /// construction, and the error does not roll back the side effects of that construction.
    pub fn apply(&self) -> Result<(), ConfigError> {
        self.validate()?;
        Python::attach(|py| -> Result<(), ConfigError> {
            // Declared before the guard so that it drops *after* the guard on
            // every exit path, including early `?` returns: locals drop in
            // reverse declaration order.
            let mut deferred_drop = super::super::DeferredDrop::default();
            let transaction = super::super::lock_runtime_transaction(py);
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
            let commit = Self::assemble_commit(
                py,
                CommitInputs {
                    before,
                    plan,
                    built,
                },
            )?;
            apply_commit(py, &commit, &mut deferred_drop)?;
            // Displaced definitions are still alive here; releasing the guard
            // before they drop keeps a Python finalizer from re-entering a
            // transaction that this thread already holds.
            drop(transaction);
            deferred_drop.release(py);
            Ok(())
        })
    }

    /// Validate handler-level filter references before any handler is built.
    fn validate_handler_filter_ids(
        &self,
        before: &RuntimeStateSnapshot,
    ) -> Result<(), ConfigError> {
        self.handlers
            .values()
            .flat_map(HandlerBuilder::filter_ids)
            .try_for_each(|id| {
                if before.filter_registry.contains_key(id) || self.filters.contains_key(id) {
                    Ok(())
                } else {
                    Err(ConfigError::UnknownIds(vec![id.clone()]))
                }
            })
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
                    .to_owned(),
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
                logger_references_overridden_id(
                    state,
                    &overridden_handler_ids,
                    &overridden_filter_ids,
                    &before.handler_filter_ids,
                )
            })
            .map(|(name, _)| name.clone())
            .collect::<BTreeSet<_>>();
        if self.root_logger.is_some() {
            impacted.insert("root".to_owned());
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
                "root".to_owned(),
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

/// Reports whether an attached handler carries a filter that this transaction overrides.
fn handler_carries_overridden_filter(
    handler_id: &str,
    handler_filter_ids: &BTreeMap<String, Vec<String>>,
    overridden_filters: &BTreeSet<String>,
) -> bool {
    handler_filter_ids
        .get(handler_id)
        .is_some_and(|ids| ids.iter().any(|id| overridden_filters.contains(id)))
}

/// Reports whether a logger's attachments reference a registry id this transaction overrides.
fn logger_references_overridden_id(
    state: &LoggerAttachmentState,
    overridden_handlers: &BTreeSet<String>,
    overridden_filters: &BTreeSet<String>,
    handler_filter_ids: &BTreeMap<String, Vec<String>>,
) -> bool {
    let overridden_handler = state
        .handler_ids()
        .iter()
        .any(|id| overridden_handlers.contains(id));
    let overridden_filter = state
        .filter_ids()
        .iter()
        .any(|id| overridden_filters.contains(id));
    let overridden_handler_filter = state.handler_ids().iter().any(|handler_id| {
        handler_carries_overridden_filter(handler_id, handler_filter_ids, overridden_filters)
    });

    overridden_handler || overridden_filter || overridden_handler_filter
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
