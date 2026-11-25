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
            // The same test-only wrapper the commit path uses, so one seam
            // covers both fallible construction sites.
            None => match construct_ancestor(py, &name) {
                Ok(logger) => logger,
                Err(error) => {
                    construction_error = Some(error);
                    break;
                }
            },
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

/// Values built before a fallible step failed, returned with the failure.
///
/// A caller that holds a guard must not let these values drop while it still
/// holds it: tearing one down may wait for a worker or run a Python finalizer
/// that needs the same lock. Returning them alongside the error is what lets
/// the caller dispose of them on an unlocked path; a bare `Result<_, PyErr>`
/// would have to drop them in place.
pub(crate) struct PartialBatch<T> {
    /// The failure that stopped the batch.
    pub(crate) error: PyErr,
    /// Values built before the failure, in request order.
    pub(crate) built: Vec<(String, T)>,
}

impl<T> std::fmt::Debug for PartialBatch<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `PyErr`'s `Debug` attaches to the interpreter, and a caller can
        // reach this `Debug` while holding the manager guard. Report only the
        // Rust-owned count; the error travels to the caller intact.
        formatter
            .debug_struct("PartialBatch")
            .field("built_len", &self.built.len())
            .finish_non_exhaustive()
    }
}

/// The error [`register_missing_ancestors`] returns.
pub(crate) type AncestorRegistrationError = PartialBatch<Py<FemtoLogger>>;

/// Build *names* with *build*, returning what was built when it fails.
///
/// The partial batch leaves the loop with the error instead of being dropped
/// by it, so a caller holding a guard can release the guard before the batch's
/// values are destroyed.
pub(crate) fn build_batch<T>(
    names: impl IntoIterator<Item = String>,
    mut build: impl FnMut(&str) -> PyResult<T>,
) -> Result<Vec<(String, T)>, PartialBatch<T>> {
    let mut built = Vec::new();
    for name in names {
        match build(&name) {
            Ok(value) => built.push((name, value)),
            Err(error) => return Err(PartialBatch { error, built }),
        }
    }
    Ok(built)
}

/// Build one ancestor logger.
///
/// This is the production path: name validation, parent resolution and
/// `Py::new` are the only ways construction can fail.
#[cfg(not(test))]
fn construct_ancestor(py: Python<'_>, name: &str) -> PyResult<Py<FemtoLogger>> {
    let parent_name = calculate_parent_name(name);
    FemtoLogger::with_parent(name.to_string(), parent_name)
        .map_err(pyo3::PyErr::from)
        .and_then(|logger| Py::new(py, logger))
}

/// Build one ancestor logger, honouring the test seam when it is set.
///
/// `register_missing_ancestors` is the only fallible step under the manager
/// write guard, and no production input makes `FemtoLogger::with_parent` fail
/// for one chosen name in a chain. Without this the non-empty partial batch
/// could never leave the guarded scope in a test, so the rule that governs it
/// would be unproven.
#[cfg(test)]
fn construct_ancestor(py: Python<'_>, name: &str) -> PyResult<Py<FemtoLogger>> {
    if let Some(failing) = injected_construction_failure()
        && failing == name
    {
        return Err(pyo3::exceptions::PyRuntimeError::new_err(
            "injected ancestor construction failure",
        ));
    }
    construct_ancestor_without_seam(py, name)
}

/// The production construction path, with the test drop-probe attached.
#[cfg(test)]
fn construct_ancestor_without_seam(py: Python<'_>, name: &str) -> PyResult<Py<FemtoLogger>> {
    let parent_name = calculate_parent_name(name);
    let logger =
        FemtoLogger::with_parent(name.to_string(), parent_name).map_err(pyo3::PyErr::from)?;
    logger
        .add_handler(std::sync::Arc::new(AncestorDropProbe))
        .map_err(pyo3::PyErr::from)?;
    Py::new(py, logger)
}

/// Whether the manager lock was free when the last ancestor probe dropped.
#[cfg(test)]
pub(crate) static ANCESTOR_DROP_SAW_FREE_LOCK: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(true);

/// How many ancestor probes have been destroyed.
#[cfg(test)]
pub(crate) static ANCESTOR_DROPS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Clear the probe counters before a test starts observing them.
///
/// The counters are process-wide, so a test resets them inside its serialised
/// section; otherwise a logger destroyed by an earlier test would be counted.
#[cfg(test)]
pub(crate) fn reset_ancestor_probe() {
    ANCESTOR_DROP_SAW_FREE_LOCK.store(true, std::sync::atomic::Ordering::SeqCst);
    ANCESTOR_DROPS.store(0, std::sync::atomic::Ordering::SeqCst);
}

/// Handler attached to every test-built ancestor to observe its destruction.
///
/// A `Py<FemtoLogger>` drop is not itself observable, but dropping the logger
/// drops its handlers, so a probe handler runs at the moment the partial batch
/// is destroyed. Recording `try_write` there is what makes the ordering claim
/// testable: if the batch were dropped under the manager guard, the probe would
/// see the lock held.
#[cfg(test)]
struct AncestorDropProbe;

#[cfg(test)]
impl crate::handler::FemtoHandlerTrait for AncestorDropProbe {
    fn handle(
        &self,
        _record: crate::log_record::FemtoLogRecord,
    ) -> Result<(), crate::handler::HandlerError> {
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
impl Drop for AncestorDropProbe {
    fn drop(&mut self) {
        ANCESTOR_DROP_SAW_FREE_LOCK.store(
            super::MANAGER.try_write().is_some(),
            std::sync::atomic::Ordering::SeqCst,
        );
        ANCESTOR_DROPS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Name whose construction the current test wants to fail, if any.
#[cfg(test)]
static INJECTED_CONSTRUCTION_FAILURE: std::sync::Mutex<Option<String>> =
    std::sync::Mutex::new(None);

/// Return the name whose construction the current test wants to fail.
#[cfg(test)]
fn injected_construction_failure() -> Option<String> {
    INJECTED_CONSTRUCTION_FAILURE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// Install or clear the test seam, returning the previous value.
///
/// The returned value lets a test restore whatever was there, so a panic
/// mid-test cannot leave the seam armed for the next one.
#[cfg(test)]
pub(crate) fn set_injected_construction_failure(name: Option<String>) -> Option<String> {
    std::mem::replace(
        &mut *INJECTED_CONSTRUCTION_FAILURE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        name,
    )
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
        return Err(PartialBatch {
            error: registry_full_error(),
            built: Vec::new(),
        });
    }
    // Build every object before inserting any: construction is fallible, and a
    // failure here must not leave a half-registered chain behind. `build_batch`
    // carries a partial result out with the error rather than dropping it under
    // the caller's write guard.
    let building: Vec<String> = novel.into_iter().filter(|name| !is_staged(name)).collect();
    let ancestors = build_batch(building, |name| {
        // The seam is a test-only wrapper around the real build, so neither the
        // hook nor the branch exists in a production build.
        construct_ancestor(py, name)
    })?;
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
