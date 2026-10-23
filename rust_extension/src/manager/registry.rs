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
use std::collections::{HashMap, HashSet};

#[cfg(feature = "python")]
mod config;

#[cfg(feature = "python")]
pub(crate) use config::{commit_staged_loggers, stage_loggers};

use super::validation::validate_logger_name;
use crate::logger::FemtoLogger;
#[cfg(feature = "python")]
use crate::{filters::FemtoFilter, handler::FemtoHandlerTrait};
#[cfg(feature = "python")]
use std::collections::BTreeMap;
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
    pub(crate) handler_filter_ids: BTreeMap<String, Vec<String>>,
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

/// Return whether the registry would fit `incoming` further named loggers.
///
/// The argument is the number to be *added*, not the total afterwards. Taking
/// the total invites the off-by-one this signature exists to prevent: every
/// caller computes `named_logger_count(&mgr) + incoming`, so a predicate that
/// also reserved a slot would refuse the ten-thousandth logger.
///
/// Kept separate from the registry so the boundary can be exercised without
/// building ten thousand loggers.
pub(super) fn registry_has_room_for(named_loggers: usize, incoming: usize) -> bool {
    named_loggers.saturating_add(incoming) <= MAX_LOGGERS
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
fn has_logger(mgr: &Manager, name: &str) -> bool {
    mgr.loggers.contains_key(name)
}

/// The dotted ancestor chain of `name`, `name` itself included.
///
/// Yields the innermost name first and stops before `"root"`: the registry's
/// own logger is created on demand and never counts against the cap, so no
/// caller needs to plan for it.
///
/// This is the unit of registration. A logger is never registered without its
/// ancestors, because propagation resolves a parent through the same
/// [`get_logger`] a caller uses, and a parent that does not exist yet is a
/// lookup that can fail later.
///
/// # Examples
///
/// ```rust,ignore
/// let chain: Vec<&str> = chain_of("api.v1.handlers").collect();
/// assert_eq!(chain, ["api.v1.handlers", "api.v1", "api"]);
/// ```
fn chain_of(name: &str) -> impl Iterator<Item = &str> {
    std::iter::successors(Some(name), |current| {
        current.rsplit_once('.').map(|(parent, _)| parent)
    })
    .take_while(|current| *current != ROOT_LOGGER_NAME)
}

/// Return the names in `names`' chains that a commit would newly register.
///
/// Ancestors are included: registering `"a.b.c"` also registers `"a"` and
/// `"a.b"`, so a commit has to account for all three. Names already in the
/// registry, repeats within `names` itself, and the exempt `"root"` are not
/// counted, and the result is ordered so an ancestor precedes its descendants.
///
/// Not gated on `python`: `get_logger` is compiled in every lane and plans
/// its own registration through this, so the same bound holds whether or not
/// the Python extension is built.
pub(crate) fn new_logger_names<'a>(
    mgr: &Manager,
    names: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut novel = Vec::new();
    for name in names {
        // Outermost first, so a caller inserting in this order creates a
        // parent before the child that names it.
        let chain: Vec<&str> = chain_of(name).collect();
        for candidate in chain.into_iter().rev() {
            if has_logger(mgr, candidate) || !seen.insert(candidate) {
                continue;
            }
            novel.push(candidate.to_string());
        }
    }
    novel
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
    if !registry_has_room_for(named_logger_count(&mgr), additions) {
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
    registry_has_room_for(named_logger_count_for_test(), 1)
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
        mgr.loggers.insert(candidate, logger);
    }

    let parent_name = calculate_parent_name(name);
    let logger = Py::new(py, FemtoLogger::with_parent(name.to_string(), parent_name)?)?;
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

#[cfg(feature = "python")]
pub(crate) fn snapshot_runtime_state() -> RuntimeStateSnapshot {
    MANAGER.read().runtime.clone()
}

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
///
/// The loggers are removed under the lock and dropped outside it. Clearing the
/// map in place would run each `FemtoLogger`'s drop — and, for a logger with
/// records still in flight, its bounded wait for the shard — while holding the
/// registry write lock, so a process resetting a full registry would serialize
/// every drop behind one lock. Taking the map out keeps the reset itself O(1)
/// under the lock and lets the drops overlap.
#[pyfunction]
pub fn reset_manager() {
    let loggers = {
        let mut mgr = MANAGER.write();
        clear_runtime_state(&mut mgr);
        std::mem::take(&mut mgr.loggers)
    };
    drop(loggers);
}
