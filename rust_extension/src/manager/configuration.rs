//! Prepared logger configuration and its single manager commit.
//!
//! The configuration builder resolves every fallible reference before calling
//! the commit. This module publishes the resolved logger states and retains
//! displaced resources until the manager lock has been released.

use std::{
    collections::{BTreeMap, hash_map::Entry},
    sync::Arc,
};

use pyo3::prelude::*;

use crate::{
    filters::FemtoFilter,
    handler::FemtoHandlerTrait,
    level::FemtoLevel,
    logger::{FemtoLogger, HandlerAttachment},
};

use super::{
    LoggerAttachmentState, MANAGER, RuntimeStateSnapshot, SharedFilters, SharedHandlers,
};

/// One logger's fully resolved configuration, ready for commit.
///
/// Every object reference is resolved during preparation so the commit takes
/// no name lookups and performs no fallible work.
pub(crate) struct PreparedLogger {
    /// Name the plan applies to.
    pub(crate) name: String,
    /// Registry handle for an existing logger, or a new unregistered object.
    pub(crate) logger: Py<FemtoLogger>,
    /// Resolved handlers to attach, replacing the current set.
    pub(crate) handlers: Vec<HandlerAttachment>,
    /// Resolved filters to attach, replacing the current set.
    pub(crate) filters: Vec<Arc<dyn FemtoFilter>>,
    /// Handler identifiers recorded as runtime attachment metadata.
    pub(crate) handler_ids: Vec<String>,
    /// Filter identifiers recorded as runtime attachment metadata.
    pub(crate) filter_ids: Vec<String>,
    /// Level to apply, or `None` to leave the current level in place.
    pub(crate) level: Option<FemtoLevel>,
    /// Propagation flag to apply, or `None` to leave the current value.
    pub(crate) propagate: Option<bool>,
}

/// The complete post-configuration state awaiting one atomic commit.
pub(crate) struct PreparedConfiguration {
    /// Logger plans, applied in order.
    pub(crate) loggers: Vec<PreparedLogger>,
    /// Whether loggers omitted from this configuration should be disabled.
    pub(crate) disable_existing: bool,
    /// Logger names and ancestors protected from disabling.
    pub(crate) keep_names: std::collections::HashSet<String>,
    /// Handler registry to publish.
    pub(crate) handler_registry: SharedHandlers,
    /// Filter IDs associated with each handler in the registry.
    pub(crate) handler_filter_ids: BTreeMap<String, Vec<String>>,
    /// Filter registry to publish.
    pub(crate) filter_registry: SharedFilters,
}

/// Resources displaced by commit and released after the manager lock.
#[derive(Default)]
pub(crate) struct DisplacedAttachments {
    /// Handler attachments removed from loggers by the commit.
    pub(crate) handlers: Vec<HandlerAttachment>,
    /// Filters removed from loggers by the commit.
    pub(crate) filters: Vec<Arc<dyn FemtoFilter>>,
    /// Staged logger handles not inserted because their names were registered
    /// by another caller before commit.
    pub(crate) unused_loggers: Vec<Py<FemtoLogger>>,
    /// The previous runtime registry snapshot, retained until after unlock.
    pub(crate) runtime_state: Option<RuntimeStateSnapshot>,
}

/// Apply a fully prepared configuration as one atomic transition.
///
/// Preparation has validated every logger name, resource reference and scalar
/// setting. This function takes the manager write lock once, performs no name
/// lookups and returns no `Result`.
///
/// The root and every logger name in the keep set are exempt from disabling.
/// Omitted targets are disabled before their attachments are cleared. The live
/// registry is consulted under the lock so registrations between preparation
/// and commit are included.
#[cfg(feature = "python")]
pub(crate) fn commit_configuration(
    py: Python<'_>,
    prepared: PreparedConfiguration,
) -> DisplacedAttachments {
    let mut displaced = DisplacedAttachments::default();
    let mut manager = MANAGER.write();
    let PreparedConfiguration {
        loggers,
        disable_existing,
        keep_names,
        handler_registry,
        handler_filter_ids,
        filter_registry,
    } = prepared;
    let mut logger_states = BTreeMap::new();

    for plan in loggers {
        let name = plan.name.clone();
        let logger = match manager.loggers.entry(name.clone()) {
            Entry::Occupied(entry) => {
                // Keep a concurrently registered logger and defer the staged
                // worker's teardown until after the manager lock is released.
                displaced.unused_loggers.push(plan.logger);
                entry.get().clone_ref(py)
            }
            Entry::Vacant(entry) => entry.insert(plan.logger).clone_ref(py),
        };
        let logger = logger.borrow(py);

        let mut old_handlers = logger.replace_handlers(plan.handlers);
        displaced.handlers.append(&mut old_handlers);
        let mut old_filters = logger.replace_filters(plan.filters);
        displaced.filters.append(&mut old_filters);
        if let Some(level) = plan.level {
            logger.set_level(level);
        }
        if let Some(propagate) = plan.propagate {
            logger.set_propagate(propagate);
        }
        // Re-enable only loggers named explicitly in this full configuration.
        logger.set_disabled(false);
        logger_states.insert(
            name,
            LoggerAttachmentState::new(plan.handler_ids, plan.filter_ids),
        );
    }

    if disable_existing {
        for (name, logger) in &manager.loggers {
            if name == "root" || keep_names.contains(name) {
                continue;
            }
            let logger = logger.borrow(py);
            logger.set_disabled(true);
            let mut old_handlers = logger.replace_handlers(Vec::new());
            displaced.handlers.append(&mut old_handlers);
            let mut old_filters = logger.replace_filters(Vec::new());
            displaced.filters.append(&mut old_filters);
        }
    }

    displaced.runtime_state = Some(std::mem::replace(
        &mut manager.runtime,
        RuntimeStateSnapshot {
            handler_registry,
            handler_filter_ids,
            filter_registry,
            logger_states,
        },
    ));
    drop(manager);
    displaced
}
