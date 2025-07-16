//! Configuration structures for [`FemtoFileHandler`].
//!
//! This module defines the various configuration types used when constructing
//! and testing file handlers. The public API exposes [`HandlerConfig`] for
//! Rust callers and [`PyHandlerConfig`] for Python bindings. Overflow handling
//! and channel capacity are also defined here so they can be shared between
//! the handler implementation and worker thread logic.

use std::{
    sync::{Arc, Barrier},
    time::Duration,
};

use pyo3::prelude::*;

/// Default bounded channel capacity for `FemtoFileHandler`.
pub const DEFAULT_CHANNEL_CAPACITY: usize = 1024;

/// Determines how `FemtoFileHandler` reacts when its queue is full.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverflowPolicy {
    /// Drop new records, preserving existing ones.
    Drop,
    /// Block the caller until space becomes available.
    Block,
    /// Block up to the specified duration before giving up.
    Timeout(Duration),
}

/// Configuration options for constructing a [`FemtoFileHandler`].
#[derive(Clone, Copy)]
pub struct HandlerConfig {
    /// Bounded queue size for records waiting to be written.
    pub capacity: usize,
    /// How often the worker thread flushes the writer.
    pub flush_interval: usize,
    /// Policy to apply when the queue is full.
    pub overflow_policy: OverflowPolicy,
}

impl Default for HandlerConfig {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_CHANNEL_CAPACITY,
            flush_interval: 1,
            overflow_policy: OverflowPolicy::Drop,
        }
    }
}

/// Configuration for `with_writer_for_test` when constructing handlers in tests.
pub struct TestConfig<W, F> {
    pub writer: W,
    pub formatter: F,
    pub capacity: usize,
    pub flush_interval: usize,
    pub overflow_policy: OverflowPolicy,
    pub start_barrier: Option<Arc<Barrier>>,
}

impl<W, F> TestConfig<W, F> {
    pub fn new(writer: W, formatter: F) -> Self {
        Self {
            writer,
            formatter,
            capacity: DEFAULT_CHANNEL_CAPACITY,
            flush_interval: 1,
            overflow_policy: OverflowPolicy::Drop,
            start_barrier: None,
        }
    }
}

/// Configuration for Python constructors requiring an overflow policy.
///
/// Groups parameters commonly used when constructing a handler so
/// `py_with_capacity_flush_policy` only accepts a single argument.
#[pyclass]
#[derive(Clone)]
pub struct PyHandlerConfig {
    /// Bounded queue size for records waiting to be written.
    /// Must be greater than zero.
    #[pyo3(get, set)]
    pub capacity: usize,
    /// How often the worker thread flushes the file.
    /// Must be greater than zero.
    #[pyo3(get, set)]
    pub flush_interval: usize,
    /// Overflow policy as a string: "drop", "block", or "timeout".
    #[pyo3(get)]
    pub policy: String,
    /// Timeout in milliseconds for the "timeout" policy.
    #[pyo3(get)]
    pub timeout_ms: Option<u64>,
}

#[pymethods]
impl PyHandlerConfig {
    #[new]
    fn new(
        capacity: usize,
        flush_interval: usize,
        policy: String,
        timeout_ms: Option<u64>,
    ) -> PyResult<Self> {
        if capacity == 0 {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "capacity must be greater than zero",
            ));
        }
        if flush_interval == 0 {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "flush_interval must be greater than zero",
            ));
        }
        let policy_lc = policy.to_ascii_lowercase();
        if !matches!(policy_lc.as_str(), "drop" | "block" | "timeout") {
            let valid = ["drop", "block", "timeout"].join(", ");
            let msg = format!("invalid overflow policy: '{policy}'. Valid options are: {valid}");
            return Err(pyo3::exceptions::PyValueError::new_err(msg));
        }
        if policy_lc != "timeout" && timeout_ms.is_some() {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "timeout_ms can only be set when policy is 'timeout'",
            ));
        }
        Ok(Self {
            capacity,
            flush_interval,
            policy: policy_lc,
            timeout_ms,
        })
    }

    #[setter]
    fn set_timeout_ms(&mut self, value: Option<u64>) -> PyResult<()> {
        if self.policy != "timeout" && value.is_some() {
            Err(pyo3::exceptions::PyValueError::new_err(
                "timeout_ms can only be set when policy is 'timeout'",
            ))
        } else {
            self.timeout_ms = value;
            Ok(())
        }
    }

    #[setter]
    fn set_policy(&mut self, value: String) -> PyResult<()> {
        let value_lc = value.to_ascii_lowercase();
        if !matches!(value_lc.as_str(), "drop" | "block" | "timeout") {
            let valid = ["drop", "block", "timeout"].join(", ");
            let msg = format!("invalid overflow policy: '{value}'. Valid options are: {valid}");
            return Err(pyo3::exceptions::PyValueError::new_err(msg));
        }
        self.policy = value_lc;
        if self.policy != "timeout" {
            self.timeout_ms = None;
        }
        Ok(())
    }
}
