//! Global registry mapping logger names to instances.
//!
//! Access is guarded by a `parking_lot::RwLock` and must only occur while the
//! Python GIL is held. This ensures `Py<FemtoLogger>` objects remain valid.

use once_cell::sync::Lazy;
use parking_lot::RwLock;
use pyo3::prelude::*;
use std::collections::{HashMap, hash_map::Entry};
#[cfg(feature = "python")]
use std::{collections::BTreeMap, sync::Arc};

use crate::logger::FemtoLogger;
#[cfg(feature = "python")]
use crate::{filters::FemtoFilter, handler::FemtoHandlerTrait};

/// Defines a private implementation contract whose behaviour is constrained by the surrounding logging runtime.
#[cfg(feature = "python")]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LoggerAttachmentState {
    /// Defines a private implementation contract whose behaviour is constrained by the surrounding logging runtime.
    handler_ids: Vec<String>,
    /// Defines a private implementation contract whose behaviour is constrained by the surrounding logging runtime.
    filter_ids: Vec<String>,
}

#[cfg(feature = "python")]
impl LoggerAttachmentState {
    /// Defines a private implementation contract whose behaviour is constrained by the surrounding logging runtime.
    pub(crate) fn new(handler_ids: Vec<String>, filter_ids: Vec<String>) -> Self {
        Self {
            handler_ids,
            filter_ids,
        }
    }

    /// Defines a private implementation contract whose behaviour is constrained by the surrounding logging runtime.
    pub(crate) fn handler_ids(&self) -> &[String] {
        &self.handler_ids
    }

    /// Defines a private implementation contract whose behaviour is constrained by the surrounding logging runtime.
    pub(crate) fn filter_ids(&self) -> &[String] {
        &self.filter_ids
    }
}

/// Defines a private implementation contract whose behaviour is constrained by the surrounding logging runtime.
#[cfg(feature = "python")]
type SharedHandlers = BTreeMap<String, Arc<dyn FemtoHandlerTrait>>;
/// Defines a private implementation contract whose behaviour is constrained by the surrounding logging runtime.
#[cfg(feature = "python")]
type SharedFilters = BTreeMap<String, Arc<dyn FemtoFilter>>;

/// Defines a private implementation contract whose behaviour is constrained by the surrounding logging runtime.
#[cfg(feature = "python")]
#[derive(Clone, Default)]
pub(crate) struct RuntimeStateSnapshot {
    /// Defines a private implementation contract whose behaviour is constrained by the surrounding logging runtime.
    pub(crate) handler_registry: SharedHandlers,
    /// Defines a private implementation contract whose behaviour is constrained by the surrounding logging runtime.
    pub(crate) filter_registry: SharedFilters,
    /// Defines a private implementation contract whose behaviour is constrained by the surrounding logging runtime.
    pub(crate) logger_states: BTreeMap<String, LoggerAttachmentState>,
}

/// Defines a private implementation contract whose behaviour is constrained by the surrounding logging runtime.
#[derive(Default)]
struct Manager {
    /// Defines a private implementation contract whose behaviour is constrained by the surrounding logging runtime.
    loggers: HashMap<String, Py<FemtoLogger>>,
    /// Defines a private implementation contract whose behaviour is constrained by the surrounding logging runtime.
    #[cfg(feature = "python")]
    runtime: RuntimeStateSnapshot,
}

/// Defines a private implementation contract whose behaviour is constrained by the surrounding logging runtime.
static MANAGER: Lazy<RwLock<Manager>> = Lazy::new(|| RwLock::new(Manager::default()));

/// Defines a private implementation contract whose behaviour is constrained by the surrounding logging runtime.
#[cfg(feature = "python")]
fn clear_runtime_state(mgr: &mut Manager) {
    mgr.runtime = RuntimeStateSnapshot::default();
}

/// Defines a private implementation contract whose behaviour is constrained by the surrounding logging runtime.
#[cfg(not(feature = "python"))]
fn clear_runtime_state(_mgr: &mut Manager) {}

/// Return `true` when the provided name is not a valid logger identifier.
///
/// A name is considered invalid when it is empty, begins or ends with a dot,
/// or contains consecutive dots which would create empty segments.
fn is_invalid_logger_name(name: &str) -> bool {
    name.is_empty()
        || name.starts_with('.')
        || name.ends_with('.')
        || name.split('.').any(|s| s.is_empty())
}

/// Defines a private implementation contract whose behaviour is constrained by the surrounding logging runtime.
fn ensure_root_logger(py: Python<'_>, mgr: &mut Manager) -> PyResult<()> {
    if !mgr.loggers.contains_key("root") {
        let root = Py::new(py, FemtoLogger::with_parent("root".into(), None))?;
        mgr.loggers.insert("root".to_string(), root);
    }
    Ok(())
}

/// Defines a private implementation contract whose behaviour is constrained by the surrounding logging runtime.
fn calculate_parent_name(name: &str) -> Option<String> {
    name.rsplit_once('.')
        .map(|(p, _)| p.to_string())
        .or_else(|| (name != "root").then(|| "root".to_string()))
}

/// Retrieve an existing logger or create one with a dotted-name parent.
pub fn get_logger(py: Python<'_>, name: &str) -> PyResult<Py<FemtoLogger>> {
    if is_invalid_logger_name(name) {
        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "logger name cannot be empty, start or end with '.', or contain consecutive dots",
        ));
    }

    let mut mgr = MANAGER.write();
    ensure_root_logger(py, &mut mgr)?;

    match mgr.loggers.entry(name.to_string()) {
        Entry::Occupied(o) => Ok(o.get().clone_ref(py)),
        Entry::Vacant(v) => {
            let parent_name = calculate_parent_name(name);
            let logger = Py::new(py, FemtoLogger::with_parent(name.to_string(), parent_name))?;
            v.insert(logger.clone_ref(py));
            Ok(logger)
        }
    }
}

/// Return an existing logger without changing the manager registry.
#[cfg(all(test, feature = "python"))]
pub(crate) fn lookup_existing_logger(py: Python<'_>, name: &str) -> PyResult<Py<FemtoLogger>> {
    if is_invalid_logger_name(name) {
        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "logger name cannot be empty, start or end with '.', or contain consecutive dots",
        ));
    }

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
            if is_invalid_logger_name(name) {
                return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                    "logger name cannot be empty, start or end with '.', or contain consecutive dots",
                ));
            }
            let logger = match mgr.loggers.get(name) {
                Some(logger) => logger.clone_ref(py),
                None => Py::new(
                    py,
                    FemtoLogger::with_parent(name.to_string(), calculate_parent_name(name)),
                )?,
            };
            Ok((name.to_string(), logger))
        })
        .collect()
}

/// Add fully staged loggers to the registry and return the committed objects.
#[cfg(feature = "python")]
pub(crate) fn commit_staged_loggers(
    py: Python<'_>,
    staged: BTreeMap<String, Py<FemtoLogger>>,
) -> BTreeMap<String, Py<FemtoLogger>> {
    let mut mgr = MANAGER.write();
    staged
        .into_iter()
        .map(|(name, logger)| {
            let committed = mgr
                .loggers
                .entry(name.clone())
                .or_insert(logger)
                .clone_ref(py);
            (name, committed)
        })
        .collect()
}

/// Clone the runtime registries while holding the manager read lock.
///
/// The returned snapshot deliberately owns Python references so configuration
/// replacement can release the global lock before it mutates handlers or
/// filters. Callers must treat it as a point-in-time view: concurrent Python
/// threads may register or detach runtime state after this lock is released.
#[cfg(feature = "python")]
pub(crate) fn snapshot_runtime_state() -> RuntimeStateSnapshot {
    MANAGER.read().runtime.clone()
}

/// Defines a private implementation contract whose behaviour is constrained by the surrounding logging runtime.
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
pub fn disable_existing_loggers(py: Python<'_>, keep_names: &std::collections::HashSet<String>) {
    let mgr = MANAGER.read();
    for (name, logger) in &mgr.loggers {
        if name != "root" && !keep_names.contains(name) {
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
    let mgr = MANAGER.read();
    for logger in mgr.loggers.values() {
        let _ = logger.borrow(py).flush_handlers();
    }
}

/// Clears cached loggers and their runtime attachments while holding the manager write lock.
///
/// The exported reset point is primarily test support; it ensures queued handler state cannot
/// survive into the next Python logging configuration.
#[pyfunction]
pub fn reset_manager() {
    let mut mgr = MANAGER.write();
    mgr.loggers.clear();
    clear_runtime_state(&mut mgr);
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
