//! Reachability analysis for handler and filter definitions.
//!
//! A published registry must only hold definitions that a logger can actually
//! reach: unreachable entries retain worker threads, queues, file descriptors,
//! and memory for the lifetime of the manager. This module holds the pure
//! identifier arithmetic that both publication paths share, so neither path has
//! to consult Python or the manager to enforce the rule.
//!
//! [`ReachableIds`] is a pair of identifier sets: the handler identifiers and
//! the filter identifiers. One type describes both the identifiers an
//! attachment graph reaches and the identifiers a transaction supplies, so the
//! two sets can be compared directly.

use std::collections::{BTreeMap, BTreeSet};

use crate::{config::ConfigError, manager::LoggerAttachmentState};

/// Handler and filter identifiers, grouped by kind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ReachableIds {
    /// Handler identifiers in the set.
    pub(crate) handlers: BTreeSet<String>,
    /// Filter identifiers in the set.
    pub(crate) filters: BTreeSet<String>,
}

impl ReachableIds {
    /// Collect the identifiers reached by committed logger attachment states.
    ///
    /// Every logger in the map contributes, not just the loggers a mutation
    /// names, so a definition shared with an untouched logger stays reachable.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let states = [
    ///     LoggerAttachmentState::new(vec!["stderr".into()], vec!["lvl".into()]),
    /// ];
    /// let reachable = ReachableIds::from_logger_states(&states);
    /// assert!(reachable.handlers.contains("stderr"));
    /// assert!(reachable.filters.contains("lvl"));
    /// ```
    pub(crate) fn from_logger_states<'a>(
        states: impl IntoIterator<Item = &'a LoggerAttachmentState>,
    ) -> Self {
        Self::from_attachment_ids(
            states
                .into_iter()
                .map(|state| (state.handler_ids(), state.filter_ids())),
        )
    }

    /// Collect logger attachments and filters referenced by attached handlers.
    ///
    /// Handler-level filters form the next edge in the runtime attachment
    /// graph. They remain reachable only while at least one logger retains
    /// their handler.
    pub(crate) fn from_logger_states_and_handler_filters<'a>(
        states: impl IntoIterator<Item = &'a LoggerAttachmentState>,
        handler_filter_ids: &BTreeMap<String, Vec<String>>,
    ) -> Self {
        let mut reachable = Self::from_logger_states(states);
        let handler_filters = reachable
            .handlers
            .iter()
            .filter_map(|handler_id| handler_filter_ids.get(handler_id))
            .flatten()
            .cloned()
            .collect::<Vec<_>>();
        reachable.filters.extend(handler_filters);
        reachable
    }

    /// Collect the identifiers from `(handler ids, filter ids)` pairs.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let handlers = vec!["stderr".to_string()];
    /// let reachable = ReachableIds::from_attachment_ids([(&handlers[..], &[][..])]);
    /// assert!(reachable.handlers.contains("stderr"));
    /// assert!(reachable.filters.is_empty());
    /// ```
    pub(crate) fn from_attachment_ids<'a>(
        attachments: impl IntoIterator<Item = (&'a [String], &'a [String])>,
    ) -> Self {
        let mut reachable = Self::default();
        for (handler_ids, filter_ids) in attachments {
            reachable.handlers.extend(handler_ids.iter().cloned());
            reachable.filters.extend(filter_ids.iter().cloned());
        }
        reachable
    }

    /// Collect the identifiers declared by a transaction.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let declared = BTreeMap::from([("stderr".to_string(), ())]);
    /// let supplied = ReachableIds::from_definition_ids(declared.keys(), [].iter());
    /// assert!(supplied.handlers.contains("stderr"));
    /// assert!(supplied.filters.is_empty());
    /// ```
    pub(crate) fn from_definition_ids<'a>(
        handler_ids: impl IntoIterator<Item = &'a String>,
        filter_ids: impl IntoIterator<Item = &'a String>,
    ) -> Self {
        Self {
            handlers: handler_ids.into_iter().cloned().collect(),
            filters: filter_ids.into_iter().cloned().collect(),
        }
    }
}

/// Drop handler registry entries that no logger attachment list mentions.
///
/// # Examples
///
/// ```rust,ignore
/// let mut registry = BTreeMap::from([("orphan".to_string(), 1)]);
/// prune_handlers(&mut registry, &ReachableIds::default());
/// assert!(registry.is_empty());
/// ```
pub(crate) fn prune_handlers<T>(registry: &mut BTreeMap<String, T>, reachable: &ReachableIds) {
    registry.retain(|id, _| reachable.handlers.contains(id));
}

/// Drop filter registry entries that no logger attachment list mentions.
///
/// # Examples
///
/// ```rust,ignore
/// let mut reachable = ReachableIds::default();
/// reachable.filters.insert("lvl".to_string());
/// let mut registry = BTreeMap::from([("lvl".to_string(), 1), ("stale".to_string(), 2)]);
/// prune_filters(&mut registry, &reachable);
/// assert_eq!(registry.keys().collect::<Vec<_>>(), ["lvl"]);
/// ```
pub(crate) fn prune_filters<T>(registry: &mut BTreeMap<String, T>, reachable: &ReachableIds) {
    registry.retain(|id, _| reachable.filters.contains(id));
}

/// Report supplied definition identifiers that no logger can reach.
///
/// Both vectors are sorted, because [`BTreeSet`] iteration is ordered, and each
/// kind is reported separately so a caller can name handlers and filters
/// distinctly. Returns `None` when every supplied definition is reachable.
///
/// # Examples
///
/// ```rust,ignore
/// let supplied = ReachableIds::from_definition_ids(["orphan".into()].iter(), [].iter());
/// let (handlers, filters) =
///     unreachable_definition_ids(&supplied, &ReachableIds::default()).expect("orphan is unreachable");
/// assert_eq!(handlers, ["orphan"]);
/// assert!(filters.is_empty());
/// ```
pub(crate) fn unreachable_definition_ids(
    supplied: &ReachableIds,
    reachable: &ReachableIds,
) -> Option<(Vec<String>, Vec<String>)> {
    let handlers = supplied
        .handlers
        .difference(&reachable.handlers)
        .cloned()
        .collect::<Vec<_>>();
    let filters = supplied
        .filters
        .difference(&reachable.filters)
        .cloned()
        .collect::<Vec<_>>();
    (!handlers.is_empty() || !filters.is_empty()).then_some((handlers, filters))
}

/// Build the error for a transaction that declared unreachable definitions.
///
/// The rejection happens before any definition is constructed, so the caller
/// never opens a file, connects a socket, or spawns a worker for a definition
/// the transaction could not have published.
///
/// # Examples
///
/// ```rust,ignore
/// let err = unreachable_definition_error(&["orphan".to_string()], &[]);
/// assert!(err.to_string().contains("unreferenced definitions"));
/// ```
pub(crate) fn unreachable_definition_error(handlers: &[String], filters: &[String]) -> ConfigError {
    let mut kinds = Vec::new();
    if !handlers.is_empty() {
        kinds.push(format!("handlers {handlers:?}"));
    }
    if !filters.is_empty() {
        kinds.push(format!("filters {filters:?}"));
    }
    ConfigError::InvalidMutation(format!(
        "unreferenced definitions: {}. Attach every definition to a logger in the same transaction, or remove it.",
        kinds.join("; "),
    ))
}
