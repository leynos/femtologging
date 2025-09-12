#![cfg(feature = "python")]
//! Shared utilities for applying handler and filter configurations to loggers.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use pyo3::prelude::PyRef;

use crate::logger::FemtoLogger;

use super::types::ConfigError;

/// Apply a sequence of items to `logger_ref`.
///
/// * `logger_ref` - logger being mutated.
/// * `ids` - ordered identifiers to resolve.
/// * `pool` - mapping of identifiers to built items.
/// * `clear` - clears existing items before attachment.
/// * `add` - attaches a resolved item to the logger.
/// * `dup_err` - constructs a duplicate identifier error.
pub(crate) fn apply_items<T: ?Sized>(
    logger_ref: &PyRef<FemtoLogger>,
    ids: &[String],
    pool: &BTreeMap<String, Arc<T>>,
    clear: impl Fn(&PyRef<FemtoLogger>),
    add: impl Fn(&PyRef<FemtoLogger>, Arc<T>),
    dup_err: impl Fn(Vec<String>) -> ConfigError,
) -> Result<(), ConfigError> {
    let mut seen = HashSet::new();
    let mut dup = Vec::new();
    let mut items = Vec::new();
    for id in ids {
        if !seen.insert(id) {
            dup.push(id.clone());
            continue;
        }
        let item = pool
            .get(id)
            .cloned()
            .ok_or_else(|| ConfigError::UnknownId(id.clone()))?;
        items.push(item);
    }
    if !dup.is_empty() {
        return Err(dup_err(dup));
    }
    clear(logger_ref);
    for item in items {
        add(logger_ref, item);
    }
    Ok(())
}
