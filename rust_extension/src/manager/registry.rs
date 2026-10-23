//! The process-global logger registry and its cardinality bound.
//!
//! The registry maps a logger name to its [`FemtoLogger`]. It never evicts: a
//! logger's handlers, level and drop counters are process state a caller may
//! still be using, so displacing one would silently redirect its records.
//! Bounding memory therefore means refusing new names once [`MAX_LOGGERS`] is
//! reached, not recycling old ones.
//!
//! Every insertion path — `get_logger`, configuration staging, and the
//! staging commit — checks the bound before it mutates anything, so a refused
//! request leaves the registry exactly as it was.
//!
//! The cap counts *named* loggers. `"root"` is exempt and is created on demand:
//! it is the fallback destination both Rust bridges resolve to, so the registry
//! must always be able to hold it whatever a caller has already registered.

use once_cell::sync::Lazy;
use parking_lot::RwLock;
use pyo3::prelude::*;
use std::collections::{HashMap, hash_map::Entry};

use super::validation::validate_logger_name;
use crate::logger::FemtoLogger;
#[cfg(feature = "python")]
use crate::{filters::FemtoFilter, handler::FemtoHandlerTrait};
#[cfg(feature = "python")]
use std::collections::{BTreeMap, HashSet};
#[cfg(feature = "python")]
use std::sync::Arc;

/// Maximum number of named loggers the registry retains.
///
/// The registry never evicts: a logger keeps its identity, its handlers and its
/// drop counters for the life of the process. The cap therefore has to be a
/// refusal rather than an eviction policy, and it bounds the memory the
/// registry itself can hold.
///
/// `"root"` sits outside this count, because it is the destination the Rust
/// bridges fall back to.
pub const MAX_LOGGERS: usize = 10_000;

/// The logger name that never counts against [`MAX_LOGGERS`].
const ROOT_LOGGER_NAME: &str = "root";

#[cfg(feature = "python")]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LoggerAttachmentState {
    handler_ids: Vec<String>,
    filter_ids: Vec<String>,
}

#[cfg(feature = "python")]
impl LoggerAttachmentState {
    pub(crate) fn new(handler_ids: Vec<String>, filter_ids: Vec<String>) -> Self {
        Self {
            handler_ids,
            filter_ids,
        }
    }

    pub(crate) fn handler_ids(&self) -> &[String] {
        &self.handler_ids
    }

    pub(crate) fn filter_ids(&self) -> &[String] {
        &self.filter_ids
    }
}

#[cfg(feature = "python")]
pub(crate) type SharedHandlers = BTreeMap<String, Arc<dyn FemtoHandlerTrait>>;
#[cfg(feature = "python")]
pub(crate) type SharedFilters = BTreeMap<String, Arc<dyn FemtoFilter>>;

#[cfg(feature = "python")]
#[derive(Clone, Default)]
pub(crate) struct RuntimeStateSnapshot {
    pub(crate) handler_registry: SharedHandlers,
    pub(crate) filter_registry: SharedFilters,
    pub(crate) logger_states: BTreeMap<String, LoggerAttachmentState>,
}

#[derive(Default)]
pub(super) struct Manager {
    pub(super) loggers: HashMap<String, Py<FemtoLogger>>,
    #[cfg(feature = "python")]
    pub(super) runtime: RuntimeStateSnapshot,
}

pub(super) static MANAGER: Lazy<RwLock<Manager>> = Lazy::new(|| RwLock::new(Manager::default()));

#[cfg(feature = "python")]
pub(super) fn clear_runtime_state(mgr: &mut Manager) {
    mgr.runtime = RuntimeStateSnapshot::default();
}

#[cfg(not(feature = "python"))]
pub(super) fn clear_runtime_state(_mgr: &mut Manager) {}

/// Return the number of registered loggers that count against the cap.
pub(super) fn named_logger_count(mgr: &Manager) -> usize {
    mgr.loggers.len() - usize::from(mgr.loggers.contains_key(ROOT_LOGGER_NAME))
}

/// Return whether one more named logger may be registered.
///
/// Kept separate from the registry so the boundary can be exercised without
/// building ten thousand loggers.
pub(super) fn registry_has_room(named_loggers: usize) -> bool {
    named_loggers < MAX_LOGGERS
}

/// Opening words of the refusal a full registry reports.
///
/// Callers that must tell a saturated registry apart from any other
/// `RuntimeError` match on this marker rather than on the whole message,
/// which embeds the cap.
pub(crate) const REGISTRY_FULL_MARKER: &str = "logger registry limit reached";

/// Build the error returned when the registry is full.
///
/// The message names the bound and says what to do instead: a caller that hits
/// this has derived names from data, and the fix is to log request-scoped
/// values as record fields rather than as logger names.
pub(crate) fn registry_full_error() -> PyErr {
    PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
        "{REGISTRY_FULL_MARKER}: at most {MAX_LOGGERS} distinct loggers are supported; \
         reuse a bounded set of component names and pass request-scoped values as record fields",
    ))
}

/// Return whether the registry already holds a logger with this name.
#[cfg(feature = "python")]
fn has_logger(mgr: &Manager, name: &str) -> bool {
    mgr.loggers.contains_key(name)
}

/// Return the names in `names` that a commit would newly register.
///
/// Names already in the registry, repeats within `names` itself, and the
/// exempt `"root"` are not counted: staging several identifiers that resolve
/// to one logger adds one entry.
///
/// Only the configuration paths need this: `get_logger` inspects a single name
/// under the write lock it already holds.
#[cfg(feature = "python")]
pub(crate) fn new_logger_names<'a>(
    mgr: &Manager,
    names: impl IntoIterator<Item = &'a str>,
) -> Vec<&'a str> {
    let mut seen = HashSet::new();
    names
        .into_iter()
        .filter(|name| *name != ROOT_LOGGER_NAME)
        .filter(|name| !has_logger(mgr, name) && seen.insert(*name))
        .collect()
}

/// Refuse a commit that would carry the registry past [`MAX_LOGGERS`].
///
/// Loggers are counted only once they are actually committed, so a large
/// configuration whose logger list is rejected here leaves no partial state
/// behind: the check runs before any insertion.
#[cfg(feature = "python")]
pub(crate) fn ensure_registry_capacity(names: &HashSet<String>) -> PyResult<()> {
    let mgr = MANAGER.read();
    let additions = new_logger_names(&mgr, names.iter().map(String::as_str)).len();
    if !registry_has_room(named_logger_count(&mgr) + additions) {
        return Err(registry_full_error());
    }
    Ok(())
}

/// Return the number of named loggers currently registered.
///
/// `"root"` is excluded, matching [`MAX_LOGGERS`], so a test can assert the
/// cap boundary directly.
#[cfg(test)]
pub(crate) fn named_logger_count_for_test() -> usize {
    named_logger_count(&MANAGER.read())
}

/// Return whether the registry would accept one more named logger.
///
/// Test-only: the boundary itself is what a bounded registry most easily gets
/// wrong, so it is worth asserting without building ten thousand loggers.
#[cfg(test)]
pub(crate) fn registry_has_room_for_test() -> bool {
    registry_has_room(named_logger_count_for_test())
}

/// Create the root logger if the registry does not already hold it.
fn ensure_root_logger(py: Python<'_>, mgr: &mut Manager) -> PyResult<()> {
    if !mgr.loggers.contains_key(ROOT_LOGGER_NAME) {
        let root = Py::new(py, FemtoLogger::with_parent(ROOT_LOGGER_NAME.into(), None)?)?;
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

/// Retrieve an existing logger or create one with a dotted-name parent.
///
/// The registry holds at most [`MAX_LOGGERS`] distinct loggers. A vacant name
/// arriving once the cap is reached is refused with a `RuntimeError` rather
/// than evicting an existing logger, because a logger's handlers, level and
/// drop counters are process state a caller may still be using.
pub fn get_logger(py: Python<'_>, name: &str) -> PyResult<Py<FemtoLogger>> {
    validate_logger_name(name)?;

    let mut mgr = MANAGER.write();
    ensure_root_logger(py, &mut mgr)?;

    // Count before taking the entry borrow: the vacant arm holds a mutable
    // borrow of `mgr` for as long as it lives, so it cannot also read `mgr`.
    let has_room = registry_has_room(named_logger_count(&mgr));

    match mgr.loggers.entry(name.to_string()) {
        Entry::Occupied(o) => Ok(o.get().clone_ref(py)),
        Entry::Vacant(v) => {
            if name != ROOT_LOGGER_NAME && !has_room {
                return Err(registry_full_error());
            }
            let parent_name = calculate_parent_name(name);
            let logger = Py::new(py, FemtoLogger::with_parent(name.to_string(), parent_name)?)?;
            v.insert(logger.clone_ref(py));
            Ok(logger)
        }
    }
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

/// Build logger objects without adding them to the global registry.
///
/// Configuration validation uses this to ensure every requested logger can be
/// created before it changes any live logging state.
#[cfg(feature = "python")]
pub(crate) fn stage_loggers<'a>(
    py: Python<'_>,
    names: impl IntoIterator<Item = &'a str>,
) -> PyResult<BTreeMap<String, Py<FemtoLogger>>> {
    let mgr = MANAGER.read();
    names
        .into_iter()
        .map(|name| {
            validate_logger_name(name)?;
            let logger = match mgr.loggers.get(name) {
                Some(logger) => logger.clone_ref(py),
                None => Py::new(
                    py,
                    FemtoLogger::with_parent(name.to_string(), calculate_parent_name(name))?,
                )?,
            };
            Ok((name.to_string(), logger))
        })
        .collect()
}

/// Add fully staged loggers to the registry and return the committed objects.
///
/// The capacity check is repeated here under the write lock, so a refused
/// commit inserts nothing: the registry cannot be left holding part of a
/// configuration that was not accepted.
#[cfg(feature = "python")]
pub(crate) fn commit_staged_loggers(
    py: Python<'_>,
    staged: BTreeMap<String, Py<FemtoLogger>>,
) -> PyResult<BTreeMap<String, Py<FemtoLogger>>> {
    let mut mgr = MANAGER.write();
    // Re-check under the write lock: another caller may have filled the last
    // slot since `stage_loggers` ran. Nothing has been inserted yet, so a
    // refusal here leaves the registry exactly as it was.
    let additions = new_logger_names(&mgr, staged.keys().map(String::as_str)).len();
    if !registry_has_room(named_logger_count(&mgr) + additions) {
        return Err(registry_full_error());
    }
    Ok(staged
        .into_iter()
        .map(|(name, logger)| {
            let committed = mgr
                .loggers
                .entry(name.clone())
                .or_insert(logger)
                .clone_ref(py);
            (name, committed)
        })
        .collect())
}

#[cfg(feature = "python")]
pub(crate) fn snapshot_runtime_state() -> RuntimeStateSnapshot {
    MANAGER.read().runtime.clone()
}

#[cfg(feature = "python")]
pub(crate) fn replace_runtime_state(
    handler_registry: SharedHandlers,
    filter_registry: SharedFilters,
    logger_states: BTreeMap<String, LoggerAttachmentState>,
) {
    let mut mgr = MANAGER.write();
    mgr.runtime = RuntimeStateSnapshot {
        handler_registry,
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

/// Flush handlers attached to every registered logger.
///
/// Intended for use by the Rust `log` crate bridge; failures are ignored.
#[cfg(feature = "log-compat")]
pub(crate) fn flush_all_handlers(py: Python<'_>) {
    let loggers = MANAGER
        .read()
        .loggers
        .values()
        .map(|logger| logger.clone_ref(py))
        .collect::<Vec<_>>();
    for logger in loggers {
        let _ = logger.borrow(py).flush_handlers();
    }
}

/// Drop every logger and all attached runtime state.
#[pyfunction]
pub fn reset_manager() {
    let mut mgr = MANAGER.write();
    mgr.loggers.clear();
    clear_runtime_state(&mut mgr);
}
