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
pub(crate) fn register_missing_ancestors<'a>(
    py: Python<'_>,
    mgr: &mut Manager,
    names: impl IntoIterator<Item = &'a str>,
    is_staged: impl Fn(&str) -> bool,
) -> PyResult<()> {
    let novel = new_logger_names(mgr, names);
    if !registry_has_room_for(named_logger_count(mgr), novel.len()) {
        return Err(registry_full_error());
    }
    // Build every object before inserting any: construction is fallible, and a
    // failure here must not leave a half-registered chain behind.
    let ancestors = novel
        .into_iter()
        .filter(|name| !is_staged(name))
        .map(|name| {
            let parent_name = calculate_parent_name(&name);
            Py::new(py, FemtoLogger::with_parent(name.clone(), parent_name)?)
                .map(|logger| (name, logger))
        })
        .collect::<PyResult<Vec<_>>>()?;
    for (name, logger) in ancestors {
        mgr.loggers.insert(name, logger);
    }
    Ok(())
}
