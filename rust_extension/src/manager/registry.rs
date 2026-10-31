//! The process-global logger registry and its cardinality bound.
//!
//! The registry maps a logger name to its [`FemtoLogger`]. It never evicts: a
//! logger's handlers, level and drop counters are process state a caller may
//! still be using, so displacing one would silently redirect its records.
//! Bounding memory therefore means refusing new names once [`MAX_LOGGERS`] is
//! reached, not recycling old ones.
//!
//! Direct `get_logger` insertion plans the full ancestor chain and checks the
//! bound under the write lock. Configuration staging also checks the complete
//! set before the staging commit rechecks and inserts it under that lock.
//! Runtime configuration instead preflights its impacted set before resolving
//! logger objects through separate `get_logger` calls. A concurrent writer can
//! consume capacity between those calls, so a late refusal may leave earlier
//! names registered even though the runtime changes were not applied.
//!
//! The cap counts *named* loggers. `"root"` is exempt and is created on demand:
//! it is the fallback destination both Rust bridges resolve to, so the registry
//! must always be able to hold it whatever a caller has already registered.

use once_cell::sync::Lazy;
use parking_lot::RwLock;
use pyo3::prelude::*;
use std::collections::HashMap;
#[cfg(feature = "python")]
use std::collections::HashSet;
#[cfg(feature = "log-compat")]
use std::sync::Arc;
#[cfg(feature = "log-compat")]
use std::sync::atomic::{AtomicU8, Ordering};

#[cfg(feature = "log-compat")]
use crate::level::FemtoLevel;

mod capacity;

#[cfg(feature = "python")]
mod config;

#[cfg(feature = "python")]
pub(crate) use config::{commit_staged_loggers, stage_loggers};

#[cfg(any(feature = "test-util", doc, test))]
pub use capacity::MAX_LOGGERS;
#[cfg(feature = "log-compat")]
pub(crate) use capacity::REGISTRY_FULL_MARKER;
pub(crate) use capacity::ROOT_LOGGER_NAME;
#[cfg(feature = "python")]
pub(crate) use capacity::ensure_registry_capacity;
pub(crate) use capacity::{
    named_logger_count, new_logger_names, registry_full_error, registry_has_room_for,
};
#[cfg(test)]
pub(crate) use capacity::{named_logger_count_for_test, registry_has_room_for_test};

#[cfg(feature = "python")]
use super::runtime_state::{
    LoggerAttachmentState, RuntimeStateSnapshot, SharedFilters, SharedHandlers,
};
use super::validation::validate_logger_name;
use crate::logger::FemtoLogger;
#[cfg(feature = "python")]
use std::collections::BTreeMap;

/// State protected by the process-wide logger registry lock.
#[derive(Default)]
pub(super) struct Manager {
    /// Strong references to loggers, keyed by their stable names.
    pub(super) loggers: HashMap<String, Py<FemtoLogger>>,
    /// Python handler, filter, and attachment state associated with the loggers.
    #[cfg(feature = "python")]
    pub(super) runtime: RuntimeStateSnapshot,
}

/// Process-wide registry lock for logger identities and runtime configuration.
pub(super) static MANAGER: Lazy<RwLock<Manager>> = Lazy::new(|| RwLock::new(Manager::default()));

/// Python-free mirror of registered logger thresholds.
///
/// The `log` bridge's enablement check must not attach to Python, so each
/// registered logger's shared level handle is indexed here when the registry
/// inserts it. The lock is always taken after [`MANAGER`] when both are held,
/// which keeps the registry writer's ordering acyclic.
#[cfg(feature = "log-compat")]
static LOGGER_LEVEL_INDEX: Lazy<RwLock<HashMap<String, Arc<AtomicU8>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

/// Add a registered logger's shared threshold to the Python-free index.
///
/// # Examples
///
/// ```ignore
/// index_logger_level(py, "service.worker", &logger);
/// ```
#[cfg(feature = "log-compat")]
pub(super) fn index_logger_level(py: Python<'_>, name: &str, logger: &Py<FemtoLogger>) {
    let level = logger.borrow(py).level_handle();
    LOGGER_LEVEL_INDEX.write().insert(name.to_string(), level);
}

/// Return a registered logger's live threshold without accessing Python.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(registered_logger_level("missing"), None);
/// ```
#[cfg(feature = "log-compat")]
pub(crate) fn registered_logger_level(name: &str) -> Option<FemtoLevel> {
    LOGGER_LEVEL_INDEX
        .read()
        .get(name)
        .map(|level| FemtoLogger::level_from_raw(level.load(Ordering::Relaxed)))
}

/// Snapshot registered levels for conservative facade maximum calculation.
///
/// # Examples
///
/// ```ignore
/// let levels = registered_logger_levels();
/// assert!(levels.iter().all(|level| *level <= FemtoLevel::Critical));
/// ```
#[cfg(feature = "log-compat")]
pub(crate) fn registered_logger_levels() -> Vec<FemtoLevel> {
    LOGGER_LEVEL_INDEX
        .read()
        .values()
        .map(|level| FemtoLogger::level_from_raw(level.load(Ordering::Relaxed)))
        .collect()
}

/// Clear the Python-specific runtime configuration while preserving loggers.
#[cfg(feature = "python")]
pub(super) fn clear_runtime_state(mgr: &mut Manager) {
    mgr.runtime = RuntimeStateSnapshot::default();
}

/// There is no Python runtime configuration to clear in this feature lane.
#[cfg(not(feature = "python"))]
pub(super) fn clear_runtime_state(_mgr: &mut Manager) {}

/// Create the root logger if the registry does not already hold it.
fn ensure_root_logger(py: Python<'_>, mgr: &mut Manager) -> PyResult<()> {
    if !mgr.loggers.contains_key(ROOT_LOGGER_NAME) {
        let root = Py::new(py, FemtoLogger::with_parent(ROOT_LOGGER_NAME.into(), None)?)?;
        #[cfg(feature = "log-compat")]
        index_logger_level(py, ROOT_LOGGER_NAME, &root);
        mgr.loggers.insert(ROOT_LOGGER_NAME.to_string(), root);
    }
    Ok(())
}

/// Derive the parent name for a dotted logger name.
fn calculate_parent_name(name: &str) -> Option<String> {
    name.rsplit_once('.')
        .map(|(p, _)| p.to_string())
        .or_else(|| (name != ROOT_LOGGER_NAME).then(|| ROOT_LOGGER_NAME.to_string()))
}

/// Retrieve an existing logger, creating it and its ancestors if needed.
///
/// Registering `"a.b.c"` registers `"a"` and `"a.b"` too: propagation resolves
/// a parent through this same function, so an ancestor left out would be a
/// lookup that fails — silently, since the propagation path ignores the error.
/// On a dotted name, the parent a logger stores is then always present.
///
/// The registry holds at most [`MAX_LOGGERS`] distinct loggers. A request whose
/// chain does not fit is refused with a `RuntimeError` *before* anything is
/// inserted, so a refusal never leaves a half-created hierarchy behind, and
/// never evicts an existing logger: a logger's handlers, level and drop
/// counters are process state a caller may still be using.
pub fn get_logger(py: Python<'_>, name: &str) -> PyResult<Py<FemtoLogger>> {
    validate_logger_name(name)?;

    // The common path, and the one every propagated record takes: a registered
    // name needs only the shared lock. Taking the write lock for it would
    // serialize every logging thread on one process-global lock for a lookup
    // that changes nothing — parent propagation calls this for each dotted
    // record.
    if let Some(logger) = MANAGER.read().loggers.get(name) {
        return Ok(logger.clone_ref(py));
    }

    let mut mgr = MANAGER.write();
    ensure_root_logger(py, &mut mgr)?;

    // Re-checked under the write lock: another caller may have registered the
    // name between the read above and this acquisition.
    if let Some(logger) = mgr.loggers.get(name) {
        return Ok(logger.clone_ref(py));
    }

    // The name is absent, so its whole chain is planned before anything is
    // built. The planner yields ancestors outermost-first; `name` itself is
    // excluded so it can be created last and returned directly rather than
    // looked up again, and so nothing here depends on the planner's ordering
    // invariant to avoid a panic.
    let ancestors: Vec<String> = new_logger_names(&mgr, std::iter::once(name))
        .into_iter()
        .filter(|candidate| candidate != name)
        .collect();
    // One check for the whole chain: a refusal must not leave a partly created
    // hierarchy behind, which it would if the bound were tested per insertion.
    // The `+ 1` is the requested name itself, which the filter above removed.
    if !registry_has_room_for(named_logger_count(&mgr), ancestors.len() + 1) {
        return Err(registry_full_error());
    }

    for candidate in ancestors {
        let parent_name = calculate_parent_name(&candidate);
        let logger = Py::new(
            py,
            FemtoLogger::with_parent(candidate.clone(), parent_name)?,
        )?;
        #[cfg(feature = "log-compat")]
        index_logger_level(py, &candidate, &logger);
        mgr.loggers.insert(candidate, logger);
    }

    let parent_name = calculate_parent_name(name);
    let logger = Py::new(py, FemtoLogger::with_parent(name.to_string(), parent_name)?)?;
    #[cfg(feature = "log-compat")]
    index_logger_level(py, name, &logger);
    mgr.loggers.insert(name.to_string(), logger.clone_ref(py));
    Ok(logger)
}

/// Return an existing logger without changing the manager registry.
#[cfg(all(test, feature = "python"))]
pub(crate) fn lookup_existing_logger(py: Python<'_>, name: &str) -> PyResult<Py<FemtoLogger>> {
    validate_logger_name(name)?;

    MANAGER
        .read()
        .loggers
        .get(name)
        .map(|logger| logger.clone_ref(py))
        .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err(format!("logger {name:?} not found")))
}

/// Copy the manager's current Python runtime configuration.
///
/// The snapshot lets callers inspect or stage changes without holding the
/// process-wide manager lock while they do so.
#[cfg(feature = "python")]
pub(crate) fn snapshot_runtime_state() -> RuntimeStateSnapshot {
    MANAGER.read().runtime.clone()
}

/// Replace the runtime registries and logger attachments as one state update.
///
/// Keeping the replacement under one write lock prevents readers from seeing
/// a mixture of the old and new configuration.
#[cfg(feature = "python")]
pub(crate) fn replace_runtime_state(
    handler_registry: SharedHandlers,
    handler_filter_ids: BTreeMap<String, Vec<String>>,
    filter_registry: SharedFilters,
    logger_states: BTreeMap<String, LoggerAttachmentState>,
) {
    let mut mgr = MANAGER.write();
    mgr.runtime = RuntimeStateSnapshot {
        handler_registry,
        handler_filter_ids,
        filter_registry,
        logger_states,
    };
}

/// Disable existing loggers not mentioned in the provided keep list.
///
/// Iterates through all loggers and clears handlers and filters for any
/// whose name is absent from `keep_names`.
#[cfg(feature = "python")]
pub fn disable_existing_loggers(py: Python<'_>, keep_names: &HashSet<String>) {
    let mgr = MANAGER.read();
    for (name, logger) in &mgr.loggers {
        if name != ROOT_LOGGER_NAME && !keep_names.contains(name) {
            let logger_ref = logger.borrow(py);
            logger_ref.clear_handlers();
            logger_ref.clear_filters();
        }
    }
}

/// Flush every registered logger for the Rust `log` bridge; ignore failures.
/// Snapshot under lock, then flush outside it: a GIL-holding `get_logger`
/// writer could otherwise starve the queued handler a flush barrier awaits.
/// `reset_manager` also drops loggers outside the lock.
#[cfg(feature = "log-compat")]
pub(crate) fn flush_all_handlers(py: Python<'_>) {
    let loggers: Vec<Py<FemtoLogger>> = MANAGER
        .read()
        .loggers
        .values()
        .map(|logger| logger.clone_ref(py))
        .collect();
    for logger in loggers {
        let _ = logger.borrow(py).flush_handlers();
    }
}

/// Drop every logger and all attached runtime state.
///
/// Take the map under lock, then drop it outside: each logger may wait for its
/// shard to drain, so clearing in place would hold the registry write lock
/// across every bounded wait and serialize a full reset.
#[pyfunction]
pub fn reset_manager() {
    let loggers = {
        let mut mgr = MANAGER.write();
        clear_runtime_state(&mut mgr);
        #[cfg(feature = "log-compat")]
        LOGGER_LEVEL_INDEX.write().clear();
        std::mem::take(&mut mgr.loggers)
    };
    // Dropping the loggers outside the registry lock may wait for queued
    // records, so refresh the facade maximum only after that drain.
    drop(loggers);
    #[cfg(feature = "log-compat")]
    crate::log_compat::refresh_global_max_level();
}
