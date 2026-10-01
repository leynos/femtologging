//! Structured logging context propagation utilities.
//!
//! This module provides a scoped, thread-local context stack and the Rust-side
//! validation used to merge context into `RecordMetadata.key_values`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use thiserror::Error;

pub(crate) const MAX_CONTEXT_KEYS: usize = 64;
pub(crate) const MAX_KEY_BYTES: usize = 64;
pub(crate) const MAX_VALUE_BYTES: usize = 1024;
pub(crate) const MAX_TOTAL_BYTES: usize = 16 * 1024;

thread_local! {
    static CONTEXT_STACK: RefCell<Vec<BTreeMap<String, String>>> = const {
        RefCell::new(Vec::new())
    };
}

/// Errors raised when validating or mutating structured logging context.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum LogContextError {
    /// The context stack was popped when no context existed.
    #[error("log context stack is empty")]
    EmptyContextStack,
    /// Too many key-values were supplied for one merged context payload.
    #[error("context has {count} keys; maximum is {max}")]
    TooManyKeys { count: usize, max: usize },
    /// A context key exceeded the byte-length limit.
    #[error("context key '{key}' is {len} bytes; maximum is {max}")]
    KeyTooLong { key: String, len: usize, max: usize },
    /// A context value exceeded the byte-length limit.
    #[error("context value for key '{key}' is {len} bytes; maximum is {max}")]
    ValueTooLong { key: String, len: usize, max: usize },
    /// Total serialized context exceeded the aggregate byte limit.
    #[error("context payload is {total} bytes; maximum is {max}")]
    TotalBytesExceeded { total: usize, max: usize },
}

#[derive(Debug, Error)]
pub(crate) enum ContextValidationError {
    #[error("context has {count} keys; maximum is {max}")]
    TooManyKeys { count: usize, max: usize },
    #[error("context key is {len} bytes; maximum is {max}")]
    KeyTooLong { len: usize, max: usize },
    #[error("context value for key '{key}' is {len} bytes; maximum is {max}")]
    ValueTooLong { key: String, len: usize, max: usize },
    #[error("context payload is {total} bytes; maximum is {max}")]
    TotalBytesExceeded { total: usize, max: usize },
}

impl ContextValidationError {
    fn into_log_context_error(self, key: &str) -> LogContextError {
        match self {
            Self::TooManyKeys { count, max } => LogContextError::TooManyKeys { count, max },
            Self::KeyTooLong { len, max } => LogContextError::KeyTooLong {
                key: key.to_owned(),
                len,
                max,
            },
            Self::ValueTooLong { key, len, max } => LogContextError::ValueTooLong { key, len, max },
            Self::TotalBytesExceeded { total, max } => {
                LogContextError::TotalBytesExceeded { total, max }
            }
        }
    }
}

/// Track the retained map's key count and aggregate UTF-8 size while building it.
#[derive(Default)]
pub(crate) struct ContextBudget {
    unique_keys: usize,
    total_bytes: usize,
}

impl ContextBudget {
    /// Validate a candidate entry before the caller retains converted strings.
    pub(crate) fn validate_next(
        &mut self,
        key: &str,
        value: &str,
        previous_value: Option<&str>,
    ) -> Result<(), ContextValidationError> {
        let key_len = key.len();
        if key_len > MAX_KEY_BYTES {
            return Err(ContextValidationError::KeyTooLong {
                len: key_len,
                max: MAX_KEY_BYTES,
            });
        }

        let is_new_key = previous_value.is_none();
        if is_new_key && self.unique_keys == MAX_CONTEXT_KEYS {
            return Err(ContextValidationError::TooManyKeys {
                count: MAX_CONTEXT_KEYS + 1,
                max: MAX_CONTEXT_KEYS,
            });
        }

        let value_len = value.len();
        if value_len > MAX_VALUE_BYTES {
            return Err(ContextValidationError::ValueTooLong {
                key: key.to_owned(),
                len: value_len,
                max: MAX_VALUE_BYTES,
            });
        }

        let next_total_bytes = self.total_bytes - previous_value.map_or(0, str::len)
            + value_len
            + if is_new_key { key_len } else { 0 };
        if next_total_bytes > MAX_TOTAL_BYTES {
            return Err(ContextValidationError::TotalBytesExceeded {
                total: next_total_bytes,
                max: MAX_TOTAL_BYTES,
            });
        }

        if is_new_key {
            self.unique_keys += 1;
        }
        self.total_bytes = next_total_bytes;
        Ok(())
    }
}

/// RAII guard that pops one context frame on drop.
#[must_use = "hold the guard for as long as the scoped log context should remain active"]
pub struct LogContextGuard {
    _private: (),
}

impl Drop for LogContextGuard {
    fn drop(&mut self) {
        let _ignored = pop_internal();
    }
}

/// Push a map-based context frame onto the current thread's context stack.
pub fn push_log_context_map(context: BTreeMap<String, String>) -> Result<(), LogContextError> {
    validate_context_map(&context)?;
    let mut merged = active_context();
    merged.extend(context.iter().map(|(k, v)| (k.clone(), v.clone())));
    validate_context_map(&merged)?;
    CONTEXT_STACK.with(|stack| stack.borrow_mut().push(context));
    Ok(())
}

/// Pop the latest context frame from the current thread's context stack.
pub fn pop_log_context() -> Result<(), LogContextError> {
    pop_internal()
}

/// Push a context frame and return a guard that pops it on drop.
pub fn push_log_context<I, K, V>(fields: I) -> Result<LogContextGuard, LogContextError>
where
    I: IntoIterator<Item = (K, V)>,
    K: Into<String>,
    V: Into<String>,
{
    let context = fields
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect::<BTreeMap<_, _>>();
    push_log_context_map(context)?;
    Ok(LogContextGuard { _private: () })
}

/// Run a closure with a pushed context frame and pop it afterward.
pub fn with_log_context<I, K, V, F, R>(fields: I, f: F) -> Result<R, LogContextError>
where
    I: IntoIterator<Item = (K, V)>,
    K: Into<String>,
    V: Into<String>,
    F: FnOnce() -> R,
{
    let _guard = push_log_context(fields)?;
    Ok(f())
}

/// Merge active scoped context into explicit key-values from a log call.
///
/// Explicit key-values always override context keys with the same name.
pub(crate) fn merge_context_values(
    explicit_key_values: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, LogContextError> {
    let stack_is_empty = CONTEXT_STACK.with(|stack| stack.borrow().is_empty());
    if stack_is_empty && explicit_key_values.is_empty() {
        return Ok(BTreeMap::new());
    }
    if stack_is_empty {
        validate_context_map(explicit_key_values)?;
        return Ok(explicit_key_values.clone());
    }

    let mut active = active_context();
    if explicit_key_values.is_empty() {
        validate_context_map(&active)?;
        return Ok(active);
    }

    active.extend(
        explicit_key_values
            .iter()
            .map(|(k, v)| (k.clone(), v.clone())),
    );
    validate_context_map(&active)?;
    Ok(active)
}

fn pop_internal() -> Result<(), LogContextError> {
    CONTEXT_STACK.with(|stack| {
        if stack.borrow_mut().pop().is_some() {
            Ok(())
        } else {
            Err(LogContextError::EmptyContextStack)
        }
    })
}

fn active_context() -> BTreeMap<String, String> {
    CONTEXT_STACK.with(|stack| {
        let mut merged = BTreeMap::new();
        for frame in stack.borrow().iter() {
            merged.extend(frame.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        merged
    })
}

fn validate_context_map(context: &BTreeMap<String, String>) -> Result<(), LogContextError> {
    if context.len() > MAX_CONTEXT_KEYS {
        return Err(LogContextError::TooManyKeys {
            count: context.len(),
            max: MAX_CONTEXT_KEYS,
        });
    }
    let mut budget = ContextBudget::default();
    for (key, value) in context {
        budget
            .validate_next(key, value, None)
            .map_err(|error| error.into_log_context_error(key))?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn clear_log_context_for_test() {
    CONTEXT_STACK.with(|stack| stack.borrow_mut().clear());
}

#[cfg(test)]
mod tests {
    //! Unit tests for scoped context propagation helpers.

    use super::*;
    use rstest::{fixture, rstest};

    #[fixture]
    fn isolated_context() {
        clear_log_context_for_test();
    }

    #[rstest]
    fn context_push_pop_round_trip(_isolated_context: ()) {
        push_log_context_map(BTreeMap::from([("request_id".into(), "123".into())]))
            .expect("context push should succeed");
        let merged = merge_context_values(&BTreeMap::new()).expect("merge should succeed");
        assert_eq!(merged.get("request_id").map(String::as_str), Some("123"));
        pop_log_context().expect("context pop should succeed");
    }

    #[rstest]
    fn nested_context_overrides_outer_keys(_isolated_context: ()) {
        push_log_context_map(BTreeMap::from([("user".into(), "outer".into())]))
            .expect("outer context should push");
        push_log_context_map(BTreeMap::from([("user".into(), "inner".into())]))
            .expect("inner context should push");
        let merged = merge_context_values(&BTreeMap::new()).expect("merge should succeed");
        assert_eq!(merged.get("user").map(String::as_str), Some("inner"));
        pop_log_context().expect("inner context should pop");
        pop_log_context().expect("outer context should pop");
    }

    #[rstest]
    fn explicit_values_override_context(_isolated_context: ()) {
        push_log_context_map(BTreeMap::from([("request_id".into(), "ctx".into())]))
            .expect("context should push");
        let explicit = BTreeMap::from([("request_id".into(), "inline".into())]);
        let merged = merge_context_values(&explicit).expect("merge should succeed");
        assert_eq!(merged.get("request_id").map(String::as_str), Some("inline"));
        pop_log_context().expect("context should pop");
    }

    #[rstest]
    fn pop_on_empty_stack_errors(_isolated_context: ()) {
        let err = pop_log_context().expect_err("empty pop should fail");
        assert_eq!(err, LogContextError::EmptyContextStack);
    }
}
