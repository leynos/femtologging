//! Staging and committing a configuration's loggers.
//!
//! A configuration names its loggers before it changes any live logging state,
//! and it must be all-or-nothing: a refused commit has to leave the registry
//! exactly as it was. That makes this path different from [`get_logger`] in
//! kind, not just in degree — it works on a *set* of names, so the capacity
//! bound has to be planned for the whole set and checked at both the staging
//! and the commit point.
//!
//! [`get_logger`]: super::get_logger

use std::collections::{BTreeMap, BTreeSet};

use pyo3::prelude::*;

#[cfg(feature = "log-compat")]
use super::index_logger_level;
use super::{
    MANAGER, Manager, calculate_parent_name, named_logger_count, new_logger_names,
    registry_full_error, registry_has_room_for,
};
use crate::logger::FemtoLogger;
use crate::manager::validation::validate_logger_name;

/// Build logger objects without adding them to the global registry.
///
/// Configuration validation uses this to ensure every requested logger can be
/// created before it changes any live logging state.
pub(crate) fn stage_loggers<'a>(
    py: Python<'_>,
    names: impl IntoIterator<Item = &'a str>,
) -> PyResult<BTreeMap<String, Py<FemtoLogger>>> {
    let requested_names: BTreeSet<String> = names
        .into_iter()
        .map(|name| {
            validate_logger_name(name)?;
            Ok(name.to_string())
        })
        .collect::<PyResult<_>>()?;
    let mgr = MANAGER.read();
    let additions = new_logger_names(&mgr, requested_names.iter().map(String::as_str));
    if !registry_has_room_for(named_logger_count(&mgr), additions.len()) {
        return Err(registry_full_error());
    }
    // Build the staged map by hand so a construction failure keeps the partial
    // map alive past the read guard. Collecting here would drop the loggers
    // built so far inside the guard, and a logger's teardown may wait for its
    // shard or run a Python finalizer that needs the manager.
    let mut staged = BTreeMap::new();
    let mut construction_error = None;
    for name in requested_names {
        let logger = match mgr.loggers.get(&name) {
            Some(logger) => logger.clone_ref(py),
            None => {
                let built = FemtoLogger::with_parent(name.clone(), calculate_parent_name(&name))
                    .map_err(pyo3::PyErr::from)
                    .and_then(|logger| Py::new(py, logger));
                match built {
                    Ok(logger) => logger,
                    Err(error) => {
                        construction_error = Some(error);
                        break;
                    }
                }
            }
        };
        staged.insert(name, logger);
    }
    drop(mgr);
    if let Some(error) = construction_error {
        drop(staged);
        return Err(error);
    }
    Ok(staged)
}

/// A failed ancestor registration, carrying what it built before failing.
///
/// Construction is fallible, and the loggers built before the failure must not
/// be destroyed while the caller still holds the manager write lock: a logger's
/// teardown may wait for its shard or run a Python finalizer that needs the
/// manager. Returning the partial batch with the error lets the caller dispose
/// of it on an unlocked path.
pub(crate) struct AncestorRegistrationError {
    /// The failure that stopped the batch.
    pub(crate) error: PyErr,
    /// Ancestors built before the failure, dropped by the caller after unlock.
    pub(crate) partial: Vec<(String, Py<FemtoLogger>)>,
}

impl std::fmt::Debug for AncestorRegistrationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The partial batch is kept out of the debug output: printing it would
        // require a Python interpreter and could run user code.
        formatter
            .debug_struct("AncestorRegistrationError")
            .field("error", &self.error)
            .field("partial_len", &self.partial.len())
            .finish()
    }
}

/// Register the missing ancestors of a configuration's loggers.
///
/// The caller holds the manager write lock, so the capacity check and the
/// insertions are one step: another writer cannot consume the room between
/// them. The check covers the whole planned set, ancestors included, and a
/// refusal happens before any insertion, so a rejected commit leaves the
/// registry exactly as it was.
///
/// Names for which `is_staged` returns true belong to the caller's own logger
/// objects and are inserted by the caller. Every other missing name in a
/// chain is created here, outermost first, so a parent always exists before
/// the child that names it. Registering the ancestors eagerly is what lets
/// propagation rely on them: a record that resolves a parent later cannot
/// discover a chain that was never completed.
///
/// `"root"` never appears here; the chain iterator stops before it, and the
/// configured root logger arrives as one of the caller's staged plans.
///
/// # Errors
///
/// Returns [`AncestorRegistrationError`] when the registry has no room or a
/// logger cannot be constructed. The error carries any ancestors built before
/// the failure so the caller can drop them after releasing the manager guard.
pub(crate) fn register_missing_ancestors<'a>(
    py: Python<'_>,
    mgr: &mut Manager,
    names: impl IntoIterator<Item = &'a str>,
    is_staged: impl Fn(&str) -> bool,
) -> Result<(), AncestorRegistrationError> {
    let novel = new_logger_names(mgr, names);
    if !registry_has_room_for(named_logger_count(mgr), novel.len()) {
        return Err(AncestorRegistrationError {
            error: registry_full_error(),
            partial: Vec::new(),
        });
    }
    // Build every object before inserting any: construction is fallible, and a
    // failure here must not leave a half-registered chain behind. The batch is
    // built by hand rather than collected so a partial result travels out with
    // the error instead of being dropped under the caller's write guard.
    let mut ancestors = Vec::with_capacity(novel.len());
    for name in novel {
        if is_staged(&name) {
            continue;
        }
        let parent_name = calculate_parent_name(&name);
        let built = FemtoLogger::with_parent(name.clone(), parent_name)
            .map_err(pyo3::PyErr::from)
            .and_then(|logger| Py::new(py, logger));
        match built {
            Ok(logger) => ancestors.push((name, logger)),
            Err(error) => {
                return Err(AncestorRegistrationError {
                    error,
                    partial: ancestors,
                });
            }
        }
    }
    for (name, logger) in ancestors {
        // The `log` bridge resolves a target's threshold without attaching to
        // Python, so an ancestor created here must be indexed exactly as
        // `get_logger` indexes the loggers it creates. The default level is
        // the facade maximum's floor, so this cannot widen the prefilter and
        // needs no refresh.
        #[cfg(feature = "log-compat")]
        index_logger_level(py, &name, &logger);
        mgr.loggers.insert(name, logger);
    }
    Ok(())
}
