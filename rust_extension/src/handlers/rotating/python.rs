//! Python bindings for rotating handler APIs.
//!
//! This module exposes Python APIs for constructing rotating file handlers with
//! configurable capacity, flush interval, overflow policy, and rotation thresholds.

use pyo3::prelude::*;

use super::{FemtoRotatingFileHandler as CoreRotatingFileHandler, RotationConfig, fresh_failure};
use crate::handlers::file::{self, DEFAULT_CHANNEL_CAPACITY, HandlerConfig};

/// Error message describing how to configure rotation thresholds.
pub const ROTATION_VALIDATION_MSG: &str =
    "both max_bytes and backup_count must be > 0 to enable rotation; set both to 0 to disable";

/// Python wrapper for the rotating file handler core type.
///
/// The wrapper keeps `PyO3` attributes out of the core module whilst preserving
/// the existing Python class name.
#[pyclass(name = "FemtoRotatingFileHandler")]
pub struct PyRotatingFileHandler {
    inner: CoreRotatingFileHandler,
}

impl PyRotatingFileHandler {
    /// Wrap a core rotating file handler for Python exposure.
    pub(crate) const fn from_core(inner: CoreRotatingFileHandler) -> Self {
        Self { inner }
    }
}

/// Python options bundling queue and rotation configuration for rotating
/// file handlers during instantiation.
///
/// The options map onto the capacity, flushing, overflow policy, and rotation
/// thresholds exposed by [`FemtoFileHandler`](crate::handlers::file::FemtoFileHandler)
/// and default to the existing values to preserve backwards compatibility.
///
/// # Examples
///
/// ```ignore
/// let options = HandlerOptions::new(
///     64,
///     2,
///     "drop".to_string(),
///     Some((1024, 3)),
/// )
/// .expect("valid options");
/// assert_eq!(options.capacity, 64);
/// assert_eq!(options.flush_interval, 2);
/// assert_eq!(options.policy, "drop");
/// assert_eq!(options.max_bytes, 1024);
/// assert_eq!(options.backup_count, 3);
/// ```
#[pyclass(from_py_object)]
#[derive(Clone)]
pub struct HandlerOptions {
    /// Queue capacity for the file handler.
    #[pyo3(get, set)]
    pub capacity: usize,
    /// Number of records written before a flush.
    #[pyo3(get, set)]
    pub flush_interval: isize,
    /// Overflow policy applied when the queue is full.
    #[pyo3(get, set)]
    pub policy: String,
    /// Maximum size of the active log file before rotation.
    #[pyo3(get, set)]
    pub max_bytes: u64,
    /// Number of rotated files retained after a rotation.
    #[pyo3(get, set)]
    pub backup_count: usize,
}

impl HandlerOptions {
    /// Validate and convert options into handler and rotation configurations.
    ///
    /// This centralizes validation logic so that both `HandlerOptions::new` and
    /// `PyRotatingFileHandler::py_new` use the same rules.
    fn to_configs(&self) -> PyResult<(HandlerConfig, RotationConfig)> {
        if (self.max_bytes == 0) != (self.backup_count == 0) {
            return Err(pyo3::exceptions::PyValueError::new_err(
                ROTATION_VALIDATION_MSG,
            ));
        }

        let capacity_input = isize::try_from(self.capacity).map_err(|_| {
            pyo3::exceptions::PyValueError::new_err("capacity must fit within isize")
        })?;
        let (validated_capacity, validated_flush_interval) = match self.flush_interval {
            -1 => file::validate_params(capacity_input, 1)?,
            value => file::validate_params(capacity_input, value)?,
        };

        let overflow_policy = file::policy::parse_policy_string(&self.policy)
            .map_err(|err| pyo3::exceptions::PyValueError::new_err(err.to_string()))?;

        let handler_cfg = HandlerConfig {
            capacity: validated_capacity,
            flush_interval: validated_flush_interval,
            overflow_policy,
        };

        let rotation = if self.max_bytes == 0 {
            RotationConfig::disabled()
        } else {
            RotationConfig::new(self.max_bytes, self.backup_count)
        };

        Ok((handler_cfg, rotation))
    }
}

#[path = "python_bindings.rs"]
mod python_bindings;

pub use python_bindings::{
    clear_rotating_fresh_failure_for_test, force_rotating_fresh_failure_for_test,
};

impl Default for HandlerOptions {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_CHANNEL_CAPACITY,
            flush_interval: 1,
            policy: "drop".to_owned(),
            max_bytes: 0,
            backup_count: 0,
        }
    }
}
