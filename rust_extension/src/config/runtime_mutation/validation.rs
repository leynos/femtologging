//! Validation helpers for runtime attachment mutations.

use std::collections::BTreeSet;

use crate::{config::ConfigError, manager::LoggerAttachmentState};

use super::CollectionMutation;

/// The identifiers a runtime transaction may attach.
///
/// Both the previously published registry identifiers and the identifiers the
/// same transaction declares are known, so planning can accept an identifier
/// before its definition has been constructed. Built registry maps and declared
/// definition maps both convert into this type.
#[derive(Clone, Debug, Default)]
pub(crate) struct KnownIds {
    /// Handler identifiers available to the planned attachment graph.
    handlers: BTreeSet<String>,
    /// Filter identifiers available to the planned attachment graph.
    filters: BTreeSet<String>,
}

impl KnownIds {
    /// Build a known-identifier set from handler and filter identifiers.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let known = KnownIds::new(["stderr".to_string()], std::iter::empty());
    /// assert!(known.contains_handler("stderr"));
    /// assert!(!known.contains_handler("stdout"));
    /// ```
    pub(crate) fn new(
        handlers: impl IntoIterator<Item = String>,
        filters: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            handlers: handlers.into_iter().collect(),
            filters: filters.into_iter().collect(),
        }
    }

    /// Build a known-identifier set from the keys of two registry maps.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let handlers = BTreeMap::from([("stderr".to_string(), handler)]);
    /// let known = KnownIds::from_registries(handlers.keys(), filters.keys());
    /// assert!(known.contains_handler("stderr"));
    /// ```
    pub(crate) fn from_registries<'a>(
        handlers: impl IntoIterator<Item = &'a String>,
        filters: impl IntoIterator<Item = &'a String>,
    ) -> Self {
        Self::new(handlers.into_iter().cloned(), filters.into_iter().cloned())
    }

    /// Extend this set with the identifiers declared by a transaction.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut known = KnownIds::from_registries([].iter(), [].iter());
    /// known.extend_definitions(["stdout".to_string()], std::iter::empty());
    /// assert!(known.contains_handler("stdout"));
    /// ```
    pub(crate) fn extend_definitions(
        &mut self,
        handlers: impl IntoIterator<Item = String>,
        filters: impl IntoIterator<Item = String>,
    ) {
        self.handlers.extend(handlers);
        self.filters.extend(filters);
    }

    /// Return whether a handler identifier is known.
    pub(crate) fn contains_handler(&self, id: &str) -> bool {
        self.handlers.contains(id)
    }

    /// Return whether a filter identifier is known.
    pub(crate) fn contains_filter(&self, id: &str) -> bool {
        self.filters.contains(id)
    }
}

/// Detects a second collection operation that changes a prior operation.
pub(crate) fn collection_conflict(
    kind: &str,
    current: &CollectionMutation,
    new: &CollectionMutation,
) -> Option<String> {
    match current {
        CollectionMutation::Unchanged => None,
        _ if current == new => None,
        _ => Some(format!(
            "conflicting mutations to the {kind} collection were requested"
        )),
    }
}

/// Reject attachment identifiers that the transaction cannot resolve.
///
/// `known` carries the pre-mutation registry identifiers *and* the identifiers
/// the same transaction declares, so an unknown identifier is one that neither
/// the manager nor the transaction can supply. Identifiers are reported in
/// handler-then-filter order, and in the order the attachment list records
/// them, which is the order existing tests rely on.
pub(crate) fn resolve_attachment_ids(
    state: &LoggerAttachmentState,
    known: &KnownIds,
) -> Result<(), ConfigError> {
    let missing_handlers = state
        .handler_ids()
        .iter()
        .filter(|id| !known.contains_handler(id));
    let missing_filters = state
        .filter_ids()
        .iter()
        .filter(|id| !known.contains_filter(id));
    let missing = missing_handlers
        .chain(missing_filters)
        .cloned()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(ConfigError::UnknownIds(missing))
    }
}

/// Ensures a removal names only attachments present in the logger's prior state.
pub(crate) fn validate_remove_ids(
    existing: &[String],
    mutation: &CollectionMutation,
) -> Result<(), ConfigError> {
    let CollectionMutation::Remove(ids) = mutation else {
        return Ok(());
    };
    let existing = existing.iter().collect::<BTreeSet<_>>();
    let missing = ids
        .iter()
        .filter(|id| !existing.contains(*id))
        .cloned()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(ConfigError::UnknownIds(missing))
    }
}
