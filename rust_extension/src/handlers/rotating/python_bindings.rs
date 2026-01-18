//! Python bindings for [`FemtoRotatingFileHandler`] and [`HandlerOptions`].
//!
//! This module exposes Python APIs for constructing rotating file handlers with
//! configurable capacity, flush interval, overflow policy, and rotation thresholds.

use pyo3::prelude::*;

use super::{FemtoRotatingFileHandler, RotationConfig, fresh_failure};
use crate::{
    formatter::DefaultFormatter,
    handlers::file::{self, DEFAULT_CHANNEL_CAPACITY, HandlerConfig},
    level::FemtoLevel,
    log_record::FemtoLogRecord,
};

/// Error message describing how to configure rotation thresholds.
pub const ROTATION_VALIDATION_MSG: &str =
    "both max_bytes and backup_count must be > 0 to enable rotation; set both to 0 to disable";

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
#[pyclass]
#[derive(Clone)]
pub struct HandlerOptions {
    #[pyo3(get, set)]
    pub capacity: usize,
    #[pyo3(get, set)]
    pub flush_interval: isize,
    #[pyo3(get, set)]
    pub policy: String,
    #[pyo3(get, set)]
    pub max_bytes: u64,
    #[pyo3(get, set)]
    pub backup_count: usize,
}

#[pymethods]
impl HandlerOptions {
    #[new]
    #[pyo3(
        text_signature = "(capacity=DEFAULT_CHANNEL_CAPACITY, flush_interval=1, policy='drop', rotation=None)"
    )]
    #[pyo3(signature = (
        capacity = DEFAULT_CHANNEL_CAPACITY,
        flush_interval = 1,
        policy = "drop".to_string(),
        rotation = None,
    ))]
    fn new(
        capacity: usize,
        flush_interval: isize,
        policy: String,
        rotation: Option<(u64, usize)>,
    ) -> PyResult<Self> {
        let (max_bytes, backup_count) = rotation.unwrap_or((0, 0));
        let flush_interval = if flush_interval == -1 {
            file::validate_params(capacity, 1)?
        } else {
            file::validate_params(capacity, flush_interval)?
        };
        let flush_interval = isize::try_from(flush_interval)
            .expect("validated flush_interval must fit within isize bounds");
        if (max_bytes == 0) != (backup_count == 0) {
            return Err(pyo3::exceptions::PyValueError::new_err(
                ROTATION_VALIDATION_MSG,
            ));
        }
        Ok(Self {
            capacity,
            flush_interval,
            policy,
            max_bytes,
            backup_count,
        })
    }
}

impl Default for HandlerOptions {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_CHANNEL_CAPACITY,
            flush_interval: 1,
            policy: "drop".to_string(),
            max_bytes: 0,
            backup_count: 0,
        }
    }
}

#[pymethods]
impl FemtoRotatingFileHandler {
    #[new]
    #[pyo3(text_signature = "(path, options=None)")]
    #[pyo3(signature = (path, options = None))]
    fn py_new(path: String, options: Option<HandlerOptions>) -> PyResult<Self> {
        let opts = options.unwrap_or_default();
        let HandlerOptions {
            capacity,
            flush_interval,
            policy,
            max_bytes,
            backup_count,
        } = opts;
        if (max_bytes == 0) != (backup_count == 0) {
            return Err(pyo3::exceptions::PyValueError::new_err(
                ROTATION_VALIDATION_MSG,
            ));
        }
        let overflow_policy = file::policy::parse_policy_string(&policy)
            .map_err(|err| pyo3::exceptions::PyValueError::new_err(err.to_string()))?;
        let flush_interval = match flush_interval {
            -1 => file::validate_params(capacity, 1)?,
            value => file::validate_params(capacity, value)?,
        };
        let handler_cfg = HandlerConfig {
            capacity,
            flush_interval,
            overflow_policy,
        };
        let rotation = if max_bytes == 0 {
            RotationConfig::disabled()
        } else {
            RotationConfig::new(max_bytes, backup_count)
        };
        Self::with_capacity_flush_policy(&path, DefaultFormatter, handler_cfg, rotation)
            .map_err(|err| pyo3::exceptions::PyIOError::new_err(format!("{path}: {err}")))
    }

    /// Expose the configured maximum number of bytes before rotation.
    #[getter]
    fn max_bytes(&self) -> u64 {
        self.rotation_limits().0
    }

    /// Expose the configured backup count.
    #[getter]
    fn backup_count(&self) -> usize {
        self.rotation_limits().1
    }

    #[pyo3(name = "handle")]
    fn py_handle(&self, logger: &str, level: &str, message: &str) -> PyResult<()> {
        let parsed_level = FemtoLevel::parse_py(level)?;
        self.handle_record(FemtoLogRecord::new(logger, parsed_level, message))
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("Handler error: {e}")))
    }

    /// Flush queued log records to disk without closing the handler or
    /// triggering a rotation.
    ///
    /// Returns
    /// -------
    /// bool
    ///     ``True`` when the worker acknowledges the flush command within the
    ///     1-second timeout.
    ///     ``False`` when the handler has already been closed, the command
    ///     cannot be delivered to the worker, or the worker fails to
    ///     acknowledge before the timeout elapses.
    ///
    /// Examples
    /// --------
    /// >>> handler.flush()
    /// True
    /// >>> handler.close()
    /// >>> handler.flush()
    /// False
    #[pyo3(name = "flush")]
    fn py_flush(&self) -> bool {
        self.flush()
    }

    #[pyo3(name = "close")]
    fn py_close(&mut self) {
        self.close();
    }
}

#[pyfunction]
pub fn force_rotating_fresh_failure_for_test(count: usize, reason: Option<&str>) -> PyResult<()> {
    let reason = reason
        .map(|value| value.to_string())
        .unwrap_or_else(|| "python requested failure".to_string());
    fresh_failure::set_forced_fresh_failure(count, reason);
    Ok(())
}

#[pyfunction]
pub fn clear_rotating_fresh_failure_for_test() {
    fresh_failure::clear_forced_fresh_failure();
}
