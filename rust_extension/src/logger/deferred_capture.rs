//! Split point for filter evaluation around a deferred traceback capture hook.
//!
//! Capture costs work proportional to an exception's object graph, and a filter
//! may reject the record before that work is justified. The producer therefore
//! splits the filter list at the first filter that can inspect traceback data:
//! everything before the split runs first, and capture only happens if that
//! prefix accepts. Filter order is never changed, so a filter that may inspect
//! `exc_info` or `stack_info` still observes it.
//!
//! The segments themselves are evaluated by the producer's handler-filter
//! module, so the split logic here stays free of filter-decision mechanics.

use std::sync::Arc;

use crate::filters::FemtoFilter;

/// Return how many leading filters cannot inspect traceback data.
///
/// The returned count is the length of the prefix that may run before capture.
/// A callback filter, or any filter that keeps the conservative default,
/// stops the prefix at its own position.
///
/// # Examples
///
/// ```rust,ignore
/// let filters: Vec<Arc<dyn FemtoFilter>> = vec![level_filter, name_filter];
/// assert_eq!(deferred_prefix(&filters), filters.len());
/// ```
pub(super) fn deferred_prefix(filters: &[Arc<dyn FemtoFilter>]) -> usize {
    filters
        .iter()
        .take_while(|filter| !filter.inspects_traceback())
        .count()
}
