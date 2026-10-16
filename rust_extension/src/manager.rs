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
mod configuration;
#[cfg(feature = "python")]
pub(crate) use configuration::{
    DisplacedAttachments, PreparedConfiguration, PreparedLogger, commit_configuration,
};
#[cfg(feature = "python")]
use crate::{filters::FemtoFilter, handler::FemtoHandlerTrait};

/// Point-in-time handler and filter identifiers attached to one logger.
#[cfg(feature = "python")]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LoggerAttachmentState {
    /// Names of registered handlers, in the logger's attachment order.
    handler_ids: Vec<String>,
    /// Names of registered filters, in the logger's attachment order.
    filter_ids: Vec<String>,
}

#[cfg(feature = "python")]
impl LoggerAttachmentState {
    /// Store attachment identifiers for a logger snapshot.
    pub(crate) fn new(handler_ids: Vec<String>, filter_ids: Vec<String>) -> Self {
        Self {
            handler_ids,
            filter_ids,
        }
    }

    /// Borrow the handler identifiers without copying the snapshot.
    pub(crate) fn handler_ids(&self) -> &[String] {
        &self.handler_ids
    }

    /// Borrow the filter identifiers without copying the snapshot.
    pub(crate) fn filter_ids(&self) -> &[String] {
        &self.filter_ids
    }
}

/// Shared handler registry retained by runtime configuration snapshots.
#[cfg(feature = "python")]
type SharedHandlers = BTreeMap<String, Arc<dyn FemtoHandlerTrait>>;
/// Shared filter registry retained by runtime configuration snapshots.
#[cfg(feature = "python")]
type SharedFilters = BTreeMap<String, Arc<dyn FemtoFilter>>;

/// Owned point-in-time view used to stage and commit Python configuration.
#[cfg(feature = "python")]
#[derive(Clone, Default)]
pub(crate) struct RuntimeStateSnapshot {
    /// Registered handlers available to staged logger attachment plans.
    pub(crate) handler_registry: SharedHandlers,
    /// Filter IDs associated with each registered handler.
    pub(crate) handler_filter_ids: BTreeMap<String, Vec<String>>,
    /// Registered filters available to staged logger attachment plans.
    pub(crate) filter_registry: SharedFilters,
    /// Per-logger attachment ID snapshots used to reconstruct configuration.
    pub(crate) logger_states: BTreeMap<String, LoggerAttachmentState>,
}

/// Global manager state protected by the registry lock.
#[derive(Default)]
struct Manager {
    /// Python logger objects keyed by their validated dotted names.
    loggers: HashMap<String, Py<FemtoLogger>>,
    /// Runtime handler/filter registries and attachment snapshots.
    #[cfg(feature = "python")]
    runtime: RuntimeStateSnapshot,
}

/// Process-wide logger registry; callers hold its lock during registry access.
static MANAGER: Lazy<RwLock<Manager>> = Lazy::new(|| RwLock::new(Manager::default()));

/// Take runtime attachment snapshots while leaving an empty registry behind.
#[cfg(feature = "python")]
fn take_runtime_state(mgr: &mut Manager) -> RuntimeStateSnapshot {
    std::mem::take(&mut mgr.runtime)
}

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

/// Ensure the implicit root logger exists before resolving named loggers.
fn ensure_root_logger(py: Python<'_>, mgr: &mut Manager) -> PyResult<()> {
    if !mgr.loggers.contains_key("root") {
        let root = Py::new(py, FemtoLogger::with_parent("root".into(), None))?;
        mgr.loggers.insert("root".to_string(), root);
    }
    Ok(())
}

/// Derive the dotted parent name, falling back to `root` for top-level names.
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

/// Validate a set of logger names without touching the registry.
///
/// Configuration preparation calls this so a malformed name is rejected before
/// any live logger state changes. The error is the same [`ValueError`] text
/// [`get_logger`] raises for a single name.
///
/// [`ValueError`]: pyo3::exceptions::PyValueError
#[cfg(feature = "python")]
pub(crate) fn validate_logger_names<'a>(
    names: impl IntoIterator<Item = &'a str>,
) -> PyResult<()> {
    for name in names {
        if is_invalid_logger_name(name) {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "logger name cannot be empty, start or end with '.', or contain consecutive dots",
            ));
        }
    }
    Ok(())
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

/// Replace runtime registries and attachment snapshots under the manager write lock.
#[cfg(feature = "python")]
pub(crate) fn replace_runtime_state(
    handler_registry: SharedHandlers,
    handler_filter_ids: BTreeMap<String, Vec<String>>,
    filter_registry: SharedFilters,
    logger_states: BTreeMap<String, LoggerAttachmentState>,
) {
    let previous = {
        let mut mgr = MANAGER.write();
        std::mem::replace(
            &mut mgr.runtime,
            RuntimeStateSnapshot {
                handler_registry,
                handler_filter_ids,
                filter_registry,
                logger_states,
            },
        )
    };
    // Handler destruction may flush output or join a worker thread.
    drop(previous);
}

/// Return all registered logger names for transaction tests.
#[cfg(all(test, feature = "python"))]
pub(crate) fn registered_logger_names_for_test() -> Vec<String> {
    let mut names = MANAGER.read().loggers.keys().cloned().collect::<Vec<_>>();
    names.sort();
    names
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

/// Clears cached loggers and runtime attachments.
///
/// The exported reset point is primarily test support. It detaches state while holding the
/// manager lock, then drops it after unlocking because worker shutdown can emit Rust log records
/// that re-enter the manager through the log compatibility bridge.
#[pyfunction]
pub fn reset_manager() {
    #[cfg(feature = "python")]
    let (loggers, runtime) = {
        let mut mgr = MANAGER.write();
        (
            std::mem::take(&mut mgr.loggers),
            take_runtime_state(&mut mgr),
        )
    };

    #[cfg(not(feature = "python"))]
    let loggers = {
        let mut mgr = MANAGER.write();
        std::mem::take(&mut mgr.loggers)
    };

    drop(loggers);
    #[cfg(feature = "python")]
    drop(runtime);
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
