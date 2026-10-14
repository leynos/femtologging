//! Structured logging context propagation utilities.
//!
//! This module provides a scoped, OS-thread-local context stack for Rust
//! emission paths. A [`LogContextGuard`] owns one frame on its creating thread
//! and removes only that frame when dropped. Rust macros and the `log` and
//! `tracing` bridges capture this context when they create a record; explicit
//! record metadata takes precedence over same-named scoped fields.
//!
//! Python logging uses a separate task-local `ContextVar`, captured when a
//! Python logging API creates its record. Python task context and Rust
//! thread-local context do not transfer implicitly across the boundary.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::{self, ThreadId};
use thiserror::Error;

#[cfg(feature = "python")]
use pyo3::prelude::*;
#[cfg(feature = "python")]
use pyo3::{PyAny, exceptions::PyModuleNotFoundError};

pub(crate) const MAX_CONTEXT_KEYS: usize = 64;
pub(crate) const MAX_KEY_BYTES: usize = 64;
pub(crate) const MAX_VALUE_BYTES: usize = 1024;
pub(crate) const MAX_TOTAL_BYTES: usize = 16 * 1024;

thread_local! {
    static CONTEXT_STACK: RefCell<Vec<ContextFrame>> = const {
        RefCell::new(Vec::new())
    };
}

static NEXT_CONTEXT_FRAME_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
struct ContextFrame {
    id: u64,
    fields: BTreeMap<String, String>,
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

/// RAII guard that removes its context frame on the owning thread when dropped.
///
/// The guard is deliberately not `Send` or `Sync`: the underlying context is
/// OS-thread-local, so a guard must stay on the thread that created it. Dropping
/// a guard on any other thread leaves both stacks unchanged.
#[must_use = "hold the guard for as long as the scoped log context should remain active"]
pub struct LogContextGuard {
    frame_id: u64,
    owner_thread: ThreadId,
    _not_send_or_sync: PhantomData<Rc<()>>,
}

impl Drop for LogContextGuard {
    fn drop(&mut self) {
        if thread::current().id() != self.owner_thread {
            return;
        }
        let _removed = remove_context_frame(self.frame_id);
    }
}

/// Push a map-based context frame onto the current thread's context stack.
pub fn push_log_context_map(context: BTreeMap<String, String>) -> Result<(), LogContextError> {
    push_context_frame(context).map(|_| ())
}

/// Pop the latest context frame from the current thread's context stack.
///
/// Prefer [`push_log_context`] for scoped use: its guard removes only the
/// frame it created, even when guards are dropped out of order.
pub fn pop_log_context() -> Result<(), LogContextError> {
    pop_internal()
}

/// Push a context frame and return a thread-affine guard that removes only that frame on drop.
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
    let frame_id = push_context_frame(context)?;
    Ok(LogContextGuard {
        frame_id,
        owner_thread: thread::current().id(),
        _not_send_or_sync: PhantomData,
    })
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

fn push_context_frame(context: BTreeMap<String, String>) -> Result<u64, LogContextError> {
    validate_context_map(&context)?;
    let mut merged = active_context();
    merged.extend(context.iter().map(|(k, v)| (k.clone(), v.clone())));
    validate_context_map(&merged)?;
    let frame_id = next_context_frame_id();
    CONTEXT_STACK.with(|stack| {
        stack.borrow_mut().push(ContextFrame {
            id: frame_id,
            fields: context,
        });
    });
    Ok(frame_id)
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

fn remove_context_frame(frame_id: u64) -> bool {
    CONTEXT_STACK.with(|stack| {
        let mut stack = stack.borrow_mut();
        let Some(position) = stack.iter().position(|frame| frame.id == frame_id) else {
            return false;
        };
        stack.remove(position);
        true
    })
}

fn next_context_frame_id() -> u64 {
    loop {
        let frame_id = NEXT_CONTEXT_FRAME_ID.fetch_add(1, Ordering::Relaxed);
        if frame_id != 0 {
            return frame_id;
        }
    }
}

fn active_context() -> BTreeMap<String, String> {
    CONTEXT_STACK.with(|stack| {
        let mut merged = BTreeMap::new();
        for frame in stack.borrow().iter() {
            merged.extend(frame.fields.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        merged
    })
}

pub(crate) fn validate_context_map(
    context: &BTreeMap<String, String>,
) -> Result<(), LogContextError> {
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

#[cfg(feature = "python")]
pub(crate) fn current_python_context(py: Python<'_>) -> PyResult<BTreeMap<String, String>> {
    let module = match py.import("femtologging._log_context") {
        Ok(module) => module,
        Err(err) if err.is_instance_of::<PyModuleNotFoundError>(py) => return Ok(BTreeMap::new()),
        Err(err) => return Err(err),
    };
    let context = module.call_method0("_current_log_context")?;
    crate::python_context::extract_python_context_map(&context)
}

#[cfg(feature = "python")]
pub(crate) fn validate_python_context(context: &Bound<'_, PyAny>) -> PyResult<()> {
    crate::python_context::extract_python_context_map(context).map(|_| ())
}

#[cfg(test)]
pub(crate) fn clear_log_context_for_test() {
    CONTEXT_STACK.with(|stack| stack.borrow_mut().clear());
}

#[cfg(test)]
#[path = "log_context_tests.rs"]
mod tests;
