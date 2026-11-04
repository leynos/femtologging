//! Prepared logger configuration and its single manager commit.
//!
//! The configuration builder resolves every fallible reference before calling
//! the commit. Under one manager write lock the commit rechecks registry
//! capacity, registers the missing ancestors of configured loggers, publishes
//! the resolved logger states, and retains displaced resources until the lock
//! and the Python GIL have been released.

use std::{
    collections::{BTreeMap, HashMap, HashSet, hash_map::Entry},
    sync::Arc,
};

use pyo3::prelude::*;

#[cfg(feature = "python")]
use crate::config::{lock_runtime_transaction, reachability::ReachableIds};
use crate::{
    filters::FemtoFilter,
    handler::FemtoHandlerTrait,
    level::FemtoLevel,
    logger::{FemtoLogger, HandlerAttachment},
};

#[cfg(feature = "log-compat")]
use super::registry::index_logger_level;
use super::{
    registry::{MANAGER, register_missing_ancestors},
    runtime_state::{LoggerAttachmentState, RuntimeStateSnapshot, SharedFilters, SharedHandlers},
};

/// One logger's fully resolved configuration, ready for commit.
///
/// Every object reference is resolved during preparation, so applying one
/// plan takes no name lookups and performs no fallible work.
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
    pub(crate) keep_names: HashSet<String>,
    /// Handler registry to publish.
    pub(crate) handler_registry: SharedHandlers,
    /// Filter IDs associated with each handler in the registry.
    pub(crate) handler_filter_ids: BTreeMap<String, Vec<String>>,
    /// Filter registry to publish.
    pub(crate) filter_registry: SharedFilters,
}

/// Resources displaced by commit and retained until its lock and GIL are released.
#[derive(Default)]
pub(crate) struct DisplacedAttachments {
    /// Handler attachments removed from loggers by the commit.
    pub(crate) handlers: Vec<HandlerAttachment>,
    /// Filters removed from loggers by the commit.
    pub(crate) filters: Vec<Arc<dyn FemtoFilter>>,
    /// Staged logger handles not inserted because their names were registered
    /// by another caller before commit.
    pub(crate) unused_loggers: Vec<Py<FemtoLogger>>,
    /// Handler definitions dropped from the published registry as unreachable.
    pub(crate) unreachable_handlers: Vec<Arc<dyn FemtoHandlerTrait>>,
    /// Filter definitions dropped from the published registry as unreachable.
    pub(crate) unreachable_filters: Vec<Arc<dyn FemtoFilter>>,
    /// The previous runtime registry snapshot, retained until after unlock.
    pub(crate) runtime_state: Option<RuntimeStateSnapshot>,
}

impl DisplacedAttachments {
    /// Drop displaced resources without the GIL after all Python borrows end.
    pub(crate) fn drop_without_gil(self, py: Python<'_>) {
        py.detach(move || drop(self));
    }
}

/// The published definition maps pruned together by one reachability pass.
///
/// They are borrowed as one unit because the handler filter-chain map must be
/// pruned in step with the handler registry: a chain that outlives its handler
/// would let a later runtime mutation resolve filters for a handler that is no
/// longer published.
struct PublishedDefinitions<'a> {
    /// Handler definitions keyed by identifier.
    handlers: &'a mut SharedHandlers,
    /// Filter identifiers associated with each handler.
    handler_filter_ids: &'a mut BTreeMap<String, Vec<String>>,
    /// Filter definitions keyed by identifier.
    filters: &'a mut SharedFilters,
}

/// Identity and registry handle used while adopting a staged logger.
struct StagedLogger<'a> {
    /// Name used as the manager registry key.
    name: &'a str,
    /// Logger object prepared before the manager lock was acquired.
    logger: Py<FemtoLogger>,
}

/// Fully resolved values that replace one logger's current state.
struct LoggerStateUpdate {
    /// Handler attachments that replace the current handlers.
    handlers: Vec<HandlerAttachment>,
    /// Filters that replace the current filters.
    filters: Vec<Arc<dyn FemtoFilter>>,
    /// Optional level update for the logger.
    level: Option<FemtoLevel>,
    /// Optional propagation update for the logger.
    propagate: Option<bool>,
}

/// Apply a fully prepared configuration as one atomic transition.
///
/// Preparation has validated every logger name, resource reference and scalar
/// setting. This function takes the runtime transaction guard, then the manager
/// write lock, so a concurrent runtime mutation cannot plan from a snapshot this
/// commit replaces. Its only fallible work is the registry capacity check and
/// the creation of missing ancestor loggers, and both happen before any live
/// logger is changed, so a refusal leaves every logger exactly as it was.
///
/// The write lock makes the capacity check authoritative: room consumed by
/// another caller after staging cannot slip in between. The planned set covers
/// every configured name and its missing ancestors, and those ancestors are
/// registered in this same commit rather than being left for a later
/// propagation lookup that can fail.
///
/// The root and every logger name in the keep set are exempt from disabling.
/// Omitted targets are disabled before their attachments are cleared. The live
/// registry is consulted under the lock so registrations between preparation
/// and commit are included.
///
/// A refusal releases the manager lock and then the GIL before the staged
/// loggers and registries are dropped: their handlers may flush or join a
/// worker during teardown, and that worker may need either.
#[cfg(feature = "python")]
pub(crate) fn commit_configuration(
    py: Python<'_>,
    prepared: PreparedConfiguration,
) -> PyResult<DisplacedAttachments> {
    let mut displaced = DisplacedAttachments::default();
    // Serialize with runtime mutations, which plan from a snapshot and would
    // otherwise overwrite this configuration with state that never contained
    // it. The transaction is taken before the manager lock so every
    // configuration path acquires the two in the same order.
    let transaction = lock_runtime_transaction(py);
    let mut manager = MANAGER.write();
    let PreparedConfiguration {
        loggers,
        disable_existing,
        keep_names,
        mut handler_registry,
        mut handler_filter_ids,
        mut filter_registry,
    } = prepared;

    let planned: HashSet<&str> = loggers.iter().map(|plan| plan.name.as_str()).collect();
    let refusal = register_missing_ancestors(py, &mut manager, planned.iter().copied(), |name| {
        planned.contains(name)
    })
    .err();
    drop(planned);
    if let Some(error) = refusal {
        // The staged loggers and registries own handlers whose drop can flush
        // or join a worker. Release the manager lock, then the GIL, before
        // those resources are torn down; dropping them under either would
        // deadlock a worker that needs the lock or the interpreter.
        drop(manager);
        drop(transaction);
        py.detach(move || drop((loggers, handler_registry, filter_registry)));
        return Err(error);
    }

    let mut logger_states = BTreeMap::new();
    for plan in loggers {
        let (name, state) = apply_prepared_logger(py, &mut manager.loggers, plan, &mut displaced);
        logger_states.insert(name, state);
    }

    if disable_existing {
        disable_omitted_loggers(py, &manager.loggers, &keep_names, &mut displaced);
    }

    // Publish only definitions a committed logger can still reach: an orphaned
    // handler would keep its worker thread, queue and file descriptor alive for
    // the manager's lifetime. Reachability considers every committed state, so a
    // definition shared with an untouched logger stays published.
    prune_unreachable_definitions(
        PublishedDefinitions {
            handlers: &mut handler_registry,
            handler_filter_ids: &mut handler_filter_ids,
            filters: &mut filter_registry,
        },
        &logger_states,
        &mut displaced,
    );

    displaced.runtime_state = Some(std::mem::replace(
        &mut manager.runtime,
        RuntimeStateSnapshot {
            handler_registry,
            handler_filter_ids,
            filter_registry,
            logger_states,
        },
    ));
    // The facade maximum is derived state over the index, so recompute it
    // before releasing the registry lock. A dispatch that observes the new
    // index thresholds must also observe the widened prefilter; refreshing
    // afterwards would leave a window in which an enabled level is filtered
    // out. Holding the manager lock here is safe: the refresh reads only the
    // index, which the established manager-then-index order already permits.
    #[cfg(feature = "log-compat")]
    crate::log_compat::refresh_global_max_level();
    drop(manager);
    // Release the transaction before the caller tears displaced resources down:
    // a Python finalizer they trigger must find the guard free.
    drop(transaction);
    Ok(displaced)
}

/// Drop handler and filter definitions that no committed logger can reach.
///
/// A handler retains its worker thread, queue and file descriptor, and a filter
/// may hold the last reference to a Python callable, so an unreachable
/// definition is a leak for the manager's lifetime. The handler filter chain map
/// is pruned with the registries so a later runtime mutation cannot resolve a
/// filter chain for a handler that is no longer published.
fn prune_unreachable_definitions(
    published: PublishedDefinitions<'_>,
    logger_states: &BTreeMap<String, LoggerAttachmentState>,
    displaced: &mut DisplacedAttachments,
) {
    let PublishedDefinitions {
        handlers: handler_registry,
        handler_filter_ids,
        filters: filter_registry,
    } = published;
    let reachable = ReachableIds::from_logger_states_and_handler_filters(
        logger_states.values(),
        handler_filter_ids,
    );
    // Removing a definition here would be its last reference, and dropping a
    // handler joins its worker while a filter may run a Python finalizer.
    // Retain a clone in `displaced` so teardown happens after the manager lock
    // and the GIL are released.
    handler_registry.retain(|handler_id, handler| {
        let is_reachable = reachable.handlers.contains(handler_id);
        if !is_reachable {
            displaced.unreachable_handlers.push(handler.clone());
        }
        is_reachable
    });
    filter_registry.retain(|filter_id, filter| {
        let is_reachable = reachable.filters.contains(filter_id);
        if !is_reachable {
            displaced.unreachable_filters.push(filter.clone());
        }
        is_reachable
    });
    handler_filter_ids.retain(|handler_id, _| reachable.handlers.contains(handler_id));
}

/// Apply one resolved plan to its adopted or newly registered logger.
fn apply_prepared_logger(
    py: Python<'_>,
    loggers: &mut HashMap<String, Py<FemtoLogger>>,
    plan: PreparedLogger,
    displaced: &mut DisplacedAttachments,
) -> (String, LoggerAttachmentState) {
    let PreparedLogger {
        name,
        logger: staged,
        handlers,
        filters,
        handler_ids,
        filter_ids,
        level,
        propagate,
    } = plan;
    let logger = adopt_logger(
        py,
        loggers,
        StagedLogger {
            name: &name,
            logger: staged,
        },
        displaced,
    );
    // The `log` bridge resolves a target's threshold without attaching to
    // Python, so every committed logger's shared level handle has to be in the
    // index. Indexing the handle is enough: a later `set_level` mutates the
    // same `Arc`, so the index never holds a stale copy.
    #[cfg(feature = "log-compat")]
    index_logger_level(py, &name, &logger);
    apply_logger_plan(
        py,
        &logger,
        LoggerStateUpdate {
            handlers,
            filters,
            level,
            propagate,
        },
        displaced,
    );

    (name, LoggerAttachmentState::new(handler_ids, filter_ids))
}

/// Insert a staged logger or retain a concurrently registered logger instead.
fn adopt_logger(
    py: Python<'_>,
    loggers: &mut HashMap<String, Py<FemtoLogger>>,
    staged: StagedLogger<'_>,
    displaced: &mut DisplacedAttachments,
) -> Py<FemtoLogger> {
    match loggers.entry(staged.name.to_owned()) {
        Entry::Occupied(entry) => {
            // Defer the unused staged worker's teardown until after unlock.
            displaced.unused_loggers.push(staged.logger);
            entry.get().clone_ref(py)
        }
        Entry::Vacant(entry) => entry.insert(staged.logger).clone_ref(py),
    }
}

/// Replace the logger's prepared state and retain displaced attachments.
fn apply_logger_plan(
    py: Python<'_>,
    logger: &Py<FemtoLogger>,
    update: LoggerStateUpdate,
    displaced: &mut DisplacedAttachments,
) {
    let LoggerStateUpdate {
        handlers,
        filters,
        level,
        propagate,
    } = update;
    let logger = logger.borrow(py);
    let mut old_handlers = logger.replace_handlers(handlers);
    displaced.handlers.append(&mut old_handlers);
    let mut old_filters = logger.replace_filters(filters);
    displaced.filters.append(&mut old_filters);
    if let Some(level) = level {
        logger.set_level(level);
    }
    if let Some(propagate) = propagate {
        logger.set_propagate(propagate);
    }
    // Re-enable only loggers named explicitly in this full configuration.
    logger.set_disabled(false);
}

/// Disable omitted loggers outside the root and keep set, clearing attachments.
fn disable_omitted_loggers(
    py: Python<'_>,
    loggers: &HashMap<String, Py<FemtoLogger>>,
    keep_names: &HashSet<String>,
    displaced: &mut DisplacedAttachments,
) {
    for (name, logger) in loggers {
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
