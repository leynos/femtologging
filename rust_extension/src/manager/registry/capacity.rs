//! Registration-count policy for the process-global logger registry.
//!
//! The registry never evicts, so bounding memory means refusing a name that
//! would carry it past [`MAX_LOGGERS`]. This module owns that arithmetic and
//! the ancestor-chain planning a refusal has to account for. Keeping the
//! policy free of the registry lock and of Python lets the boundary be
//! exercised without building ten thousand loggers.

use std::collections::HashSet;

use pyo3::prelude::*;

#[cfg(any(feature = "python", test))]
use super::MANAGER;
use super::Manager;

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
pub(crate) const ROOT_LOGGER_NAME: &str = "root";

/// Return the number of registered loggers that count against the cap.
pub(crate) fn named_logger_count(mgr: &Manager) -> usize {
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
pub(crate) fn registry_has_room_for(named_loggers: usize, incoming: usize) -> bool {
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
/// [`get_logger`](super::get_logger) a caller uses, and a parent that does not
/// exist yet is a lookup that can fail later.
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
