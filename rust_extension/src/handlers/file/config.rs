//! Configuration structures and Python bindings for [`FemtoFileHandler`].
//!
//! This module defines the various configuration types used when constructing
//! and testing file handlers. The public API exposes [`HandlerConfig`] for Rust
//! callers and [`PyHandlerConfig`] for Python bindings. Overflow handling and
//! channel capacity are also defined here so they can be shared between the
//! handler implementation and worker thread logic.

use std::{
    sync::{Arc, Barrier},
    time::Duration,
};

use pyo3::prelude::*;

/// Default bounded channel capacity for `FemtoFileHandler`.
pub const DEFAULT_CHANNEL_CAPACITY: usize = 1024;
const VALID_POLICIES: [&str; 3] = ["drop", "block", "timeout"];

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
    #[pyo3(get)]
    pub capacity: usize,
    /// How often the worker thread flushes the file.
    /// Must be greater than zero.
    #[pyo3(get)]
    pub flush_interval: usize,
    /// Overflow policy as a string: "drop", "block", or "timeout".
    #[pyo3(get)]
    pub policy: String,
    /// Timeout in milliseconds for the "timeout" policy.
    /// Must be greater than zero when set.
    #[pyo3(get)]
    pub timeout_ms: Option<u64>,
}

#[pymethods]
impl PyHandlerConfig {
    /// Ensure a value is greater than zero.
    #[staticmethod]
    fn validate_positive(value: usize, field: &str) -> PyResult<()> {
        if value == 0 {
            Err(pyo3::exceptions::PyValueError::new_err(format!(
                "{field} must be greater than zero"
            )))
        } else {
            Ok(())
        }
    }

    /// Validate the overflow policy and optional timeout.
    #[staticmethod]
    fn validate_policy(policy: &str, timeout_ms: Option<u64>) -> PyResult<()> {
        // Trim and ignore case so callers may pass mixed-case policies.
        let candidate = policy.trim();
        if !VALID_POLICIES
            .iter()
            .any(|valid| valid.eq_ignore_ascii_case(candidate))
        {
            let valid = VALID_POLICIES.join(", ");
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "invalid overflow policy: '{candidate}'. Valid options are: {valid}"
            )));
        }
        if "timeout".eq_ignore_ascii_case(candidate) {
            match timeout_ms {
                Some(ms) if ms > 0 => Ok(()),
                Some(_) => Err(pyo3::exceptions::PyValueError::new_err(
                    "timeout_ms must be greater than zero",
                )),
                None => Err(pyo3::exceptions::PyValueError::new_err(
                    "timeout_ms required when policy is 'timeout'",
                )),
            }
        } else if timeout_ms.is_some() {
            Err(pyo3::exceptions::PyValueError::new_err(
                "timeout_ms can only be set when policy is 'timeout'",
            ))
        } else {
            Ok(())
        }
    }
    #[new]
    fn new(
        capacity: usize,
        flush_interval: usize,
        policy: String,
        timeout_ms: Option<u64>,
    ) -> PyResult<Self> {
        Self::validate_positive(capacity, "capacity")?;
        Self::validate_positive(flush_interval, "flush_interval")?;
        Self::validate_policy(&policy, timeout_ms)?;
        let policy_lc = policy.trim().to_ascii_lowercase();
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
        } else if self.policy == "timeout" && value.is_none() {
            Err(pyo3::exceptions::PyValueError::new_err(
                "timeout_ms required when policy is 'timeout'",
            ))
        } else if matches!(value, Some(ms) if ms == 0) {
            Err(pyo3::exceptions::PyValueError::new_err(
                "timeout_ms must be greater than zero",
            ))
        } else {
            self.timeout_ms = value;
            Ok(())
        }
    }

    #[setter]
    fn set_capacity(&mut self, value: usize) -> PyResult<()> {
        Self::validate_positive(value, "capacity")?;
        self.capacity = value;
        Ok(())
    }

    #[setter]
    fn set_flush_interval(&mut self, value: usize) -> PyResult<()> {
        Self::validate_positive(value, "flush_interval")?;
        self.flush_interval = value;
        Ok(())
    }

    /// Atomically switch to the TIMEOUT policy with a required, non-zero
    /// timeout.
    ///
    /// # Examples
    ///
    /// ```python
    /// cfg = PyHandlerConfig(1, 1, "drop", timeout_ms=None)
    /// cfg.set_policy_timeout(250)
    /// assert cfg.policy == "timeout"
    /// assert cfg.timeout_ms == 250
    /// ```
    pub fn set_policy_timeout(&mut self, timeout_ms: u64) -> PyResult<()> {
        Self::validate_policy("timeout", Some(timeout_ms))?;
        self.policy = "timeout".to_string();
        self.timeout_ms = Some(timeout_ms);
        Ok(())
    }

    #[setter]
    fn set_policy(&mut self, value: String) -> PyResult<()> {
        Self::validate_policy(&value, self.timeout_ms)?;
        self.policy = value.trim().to_ascii_lowercase();
        if self.policy != "timeout" {
            self.timeout_ms = None;
        }
        Ok(())
    }
}
