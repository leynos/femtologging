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

use std::collections::{BTreeMap, BTreeSet, HashSet};

use pyo3::prelude::*;

use super::{
    MANAGER, calculate_parent_name, named_logger_count, new_logger_names, registry_full_error,
    registry_has_room_for,
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
    requested_names
        .into_iter()
        .map(|name| {
            let logger = match mgr.loggers.get(&name) {
                Some(logger) => logger.clone_ref(py),
                None => Py::new(
                    py,
                    FemtoLogger::with_parent(name.clone(), calculate_parent_name(&name))?,
                )?,
            };
            Ok((name, logger))
        })
        .collect()
}

/// Add fully staged loggers to the registry and return the committed objects.
///
/// The capacity check is repeated here under the write lock, so a refused
/// commit inserts nothing: the registry cannot be left holding part of a
/// configuration that was not accepted.
pub(crate) fn commit_staged_loggers(
    py: Python<'_>,
    staged: BTreeMap<String, Py<FemtoLogger>>,
) -> PyResult<BTreeMap<String, Py<FemtoLogger>>> {
    let mut mgr = MANAGER.write();
    // Re-check under the write lock: another caller may have filled the last
    // slot since `stage_loggers` ran. Nothing has been inserted yet, so a
    // refusal here leaves the registry exactly as it was. The count covers
    // each staged chain, so committing `"a.b"` accounts for `"a"` as well.
    let additions = new_logger_names(&mgr, staged.keys().map(String::as_str));
    if !registry_has_room_for(named_logger_count(&mgr), additions.len()) {
        return Err(registry_full_error());
    }
    // An ancestor a configuration reaches through but does not itself
    // configure still has to exist: propagation resolves a parent through the
    // registry, and a missing one would silently stop a child's records
    // reaching root. Only those are built here — a name the configuration does
    // configure is committed from its staged object below, which carries the
    // handlers and filters the plan already applied to it.
    let configured: HashSet<&str> = staged.keys().map(String::as_str).collect();
    for name in additions
        .into_iter()
        .filter(|name| !configured.contains(name.as_str()))
    {
        let parent_name = calculate_parent_name(&name);
        let logger = Py::new(py, FemtoLogger::with_parent(name.clone(), parent_name)?)?;
        mgr.loggers.insert(name, logger);
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
