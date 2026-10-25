//! Builder for structured runtime mutation of a single logger.
//!
//! The builder keeps scalar changes (`level`, `propagate`) separate from
//! collection changes so handlers and filters can be appended, replaced,
//! removed, or cleared explicitly. Conflicting collection modes are recorded
//! rather than raised, so the error is reported against the logger name at
//! validation time.

use pyo3::prelude::*;

use crate::{FemtoLevel, config::ConfigError};

use super::{CollectionMutation, collection_conflict};

#[cfg_attr(feature = "python", pyclass(from_py_object))]
#[derive(Clone, Debug, Default)]
pub struct LoggerMutationBuilder {
    /// Optional level override applied to the logger.
    pub(crate) level: Option<FemtoLevel>,
    /// Optional propagation override applied to the logger.
    pub(crate) propagate: Option<bool>,
    /// Requested change to the logger's handler attachments.
    pub(crate) handlers: CollectionMutation,
    /// Requested change to the logger's filter attachments.
    pub(crate) filters: CollectionMutation,
    /// First conflicting collection change, reported during validation.
    invalid: Option<String>,
}

impl LoggerMutationBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Convert string-like identifiers into owned strings.
    fn normalize_ids<I, S>(ids: I) -> Vec<String>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        ids.into_iter().map(Into::into).collect()
    }

    /// Build and record a collection mutation from string-like identifiers.
    fn apply_ids_mutation<I, S>(
        mut self,
        ids: I,
        mutation: impl FnOnce(Vec<String>) -> CollectionMutation,
        set: impl FnOnce(&mut Self, CollectionMutation),
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let ids = Self::normalize_ids(ids);
        set(&mut self, mutation(ids));
        self
    }

    /// Record a replacement operation through the supplied collection setter.
    fn do_replace<I, S>(self, ids: I, setter: impl FnOnce(&mut Self, CollectionMutation)) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.apply_ids_mutation(ids, CollectionMutation::replace, setter)
    }

    /// Record an append operation through the supplied collection setter.
    fn do_append<I, S>(self, ids: I, setter: impl FnOnce(&mut Self, CollectionMutation)) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.apply_ids_mutation(ids, CollectionMutation::append, setter)
    }

    /// Record a removal operation through the supplied collection setter.
    fn do_remove<I, S>(self, ids: I, setter: impl FnOnce(&mut Self, CollectionMutation)) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.apply_ids_mutation(ids, CollectionMutation::remove, setter)
    }

    /// Record a clear operation through the supplied collection setter.
    fn do_clear(self, setter: impl FnOnce(&mut Self, CollectionMutation)) -> Self {
        let mut this = self;
        setter(&mut this, CollectionMutation::Clear);
        this
    }

    pub fn with_level(mut self, level: FemtoLevel) -> Self {
        self.level = Some(level);
        self
    }

    pub fn with_propagate(mut self, propagate: bool) -> Self {
        self.propagate = Some(propagate);
        self
    }

    pub fn replace_handlers<I, S>(self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.do_replace(ids, Self::set_handlers)
    }

    pub fn append_handlers<I, S>(self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.do_append(ids, Self::set_handlers)
    }

    pub fn remove_handlers<I, S>(self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.do_remove(ids, Self::set_handlers)
    }

    pub fn clear_handlers(self) -> Self {
        self.do_clear(Self::set_handlers)
    }

    pub fn replace_filters<I, S>(self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.do_replace(ids, Self::set_filters)
    }

    pub fn append_filters<I, S>(self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.do_append(ids, Self::set_filters)
    }

    pub fn remove_filters<I, S>(self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.do_remove(ids, Self::set_filters)
    }

    pub fn clear_filters(self) -> Self {
        self.do_clear(Self::set_filters)
    }

    /// Store a handler change and remember any conflict with an earlier change.
    pub(crate) fn set_handlers(&mut self, mutation: CollectionMutation) {
        if self.invalid.is_none() {
            self.invalid = collection_conflict("handler", &self.handlers, &mutation);
        }
        self.handlers = mutation;
    }

    /// Store a filter change and remember any conflict with an earlier change.
    pub(crate) fn set_filters(&mut self, mutation: CollectionMutation) {
        if self.invalid.is_none() {
            self.invalid = collection_conflict("filter", &self.filters, &mutation);
        }
        self.filters = mutation;
    }

    /// Return a recorded collection conflict with the logger name attached.
    pub(crate) fn ensure_valid(&self, logger_name: &str) -> Result<(), ConfigError> {
        self.invalid
            .clone()
            .map(|message| ConfigError::InvalidMutation(format!("{logger_name}: {message}")))
            .map_or(Ok(()), Err)
    }
}
