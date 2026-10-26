//! Python bindings for rotating handler APIs.
//!
//! This module exposes Python APIs for constructing rotating file handlers with
//! configurable capacity, flush interval, overflow policy, and rotation thresholds.

use std::any::Any;

use pyo3::prelude::*;

use super::{FemtoRotatingFileHandler as CoreRotatingFileHandler, RotationConfig, fresh_failure};
use crate::{
    formatter::DefaultFormatter,
    handler::FemtoHandlerTrait,
    handlers::file::{self, DEFAULT_CHANNEL_CAPACITY, HandlerConfig},
    level::FemtoLevel,
    log_record::FemtoLogRecord,
};

/// Error message describing how to configure rotation thresholds.
pub const ROTATION_VALIDATION_MSG: &str =
    "both max_bytes and backup_count must be > 0 to enable rotation; set both to 0 to disable";

/// Python wrapper for the rotating file handler core type.
///
/// The wrapper keeps PyO3 attributes out of the core module whilst preserving
/// the existing Python class name.
#[pyclass(name = "FemtoRotatingFileHandler")]
pub struct PyRotatingFileHandler {
    /// Core handler that owns the file worker and rotation state.
    inner: CoreRotatingFileHandler,
}

impl PyRotatingFileHandler {
    /// Wrap a core rotating file handler for Python exposure.
    pub(crate) fn from_core(inner: CoreRotatingFileHandler) -> Self {
        Self { inner }
    }
}

impl FemtoHandlerTrait for PyRotatingFileHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), crate::handler::HandlerError> {
        self.inner.handle(record)
    }

    fn flush(&self) -> bool {
        self.inner.flush()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Python value object carrying queue and rotation options for a rotating file handler.
///
/// `rotation=None` disables rotation. Otherwise, the pair contains the maximum
/// file size in bytes and the number of backup files to keep; both values must
/// be greater than zero.
#[pyclass(from_py_object)]
#[derive(Clone)]
pub struct HandlerOptions {
    /// Maximum number of records waiting in the handler's queue.
    #[pyo3(get, set)]
    pub capacity: usize,
    /// Flush the file after this many records; the value must be greater than zero.
    #[pyo3(get, set)]
    pub flush_interval: isize,
    /// Queue overflow policy: `drop`, `block`, or `timeout:N` in milliseconds.
    #[pyo3(get, set)]
    pub policy: String,
    /// Maximum file size in bytes before rotation; zero disables rotation.
    #[pyo3(get, set)]
    pub max_bytes: u64,
    /// Number of rotated backup files to retain; zero disables rotation.
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

        let capacity = isize::try_from(self.capacity).map_err(|_| {
            pyo3::exceptions::PyValueError::new_err("capacity must fit within isize")
        })?;
        let (capacity, flush_interval) = match self.flush_interval {
            -1 => file::validate_params(capacity, 1)?,
            value => file::validate_params(capacity, value)?,
        };

        let overflow_policy = file::policy::parse_policy_string(&self.policy)
            .map_err(|err| pyo3::exceptions::PyValueError::new_err(err.to_string()))?;

        let handler_cfg = HandlerConfig {
            capacity,
            flush_interval,
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

#[pymethods]
impl HandlerOptions {
    /// Create validated queue and rotation options.
    ///
    /// Pass `None` for `rotation` to disable rotation, or provide a
    /// `(max_bytes, backup_count)` pair with both values greater than zero.
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
    /// assert_eq!(options.max_bytes, 1024);
    /// ```
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

        let tmp = Self {
            capacity,
            flush_interval,
            policy,
            max_bytes,
            backup_count,
        };

        // Validate using the same logic as py_new; discard configs, keep validated opts
        let _ = tmp.to_configs()?;
        Ok(tmp)
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
impl PyRotatingFileHandler {
    /// Open a rotating file handler with the supplied options.
    ///
    /// File writes, rotation and worker shutdown are serialized by the handler.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let handler = PyRotatingFileHandler::py_new("service.log".to_owned(), None)?;
    /// ```
    #[new]
    #[pyo3(text_signature = "(path, options=None)")]
    #[pyo3(signature = (path, options = None))]
    fn py_new(path: String, options: Option<HandlerOptions>) -> PyResult<Self> {
        let opts = options.unwrap_or_default();
        let (handler_cfg, rotation) = opts.to_configs()?;

        CoreRotatingFileHandler::with_capacity_flush_policy(
            &path,
            DefaultFormatter,
            handler_cfg,
            rotation,
        )
        .map(Self::from_core)
        .map_err(|err| pyo3::exceptions::PyIOError::new_err(format!("{path}: {err}")))
    }

    /// Expose the configured maximum number of bytes before rotation.
    #[getter]
    fn max_bytes(&self) -> u64 {
        self.inner.rotation_limits().0
    }

    /// Expose the configured backup count.
    #[getter]
    fn backup_count(&self) -> usize {
        self.inner.rotation_limits().1
    }

    /// Send one record to the rotating file handler.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// handler.py_handle("service", "INFO", "started")?;
    /// ```
    #[pyo3(name = "handle")]
    fn py_handle(&self, logger: &str, level: &str, message: &str) -> PyResult<()> {
        let parsed_level = FemtoLevel::parse_py(level)?;
        self.inner
            .handle(FemtoLogRecord::new(logger, parsed_level, message))
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("Handler error: {e}")))
    }

    /// Flush queued log records to disk without closing the handler or
    /// triggering a rotation.
    ///
    /// Uses a fixed 1-second timeout.
    ///
    /// Returns
    /// -------
    /// bool
    ///     ``True`` when the worker acknowledges the flush within the
    ///     timeout.
    ///     ``False`` when the handler has already been closed, the
    ///     internal channel to the worker has been dropped, or the worker
    ///     does not acknowledge before the timeout elapses.
    ///
    /// Examples
    /// --------
    /// >>> handler.flush()
    /// True
    /// >>> handler.close()
    /// >>> handler.flush()
    /// False
    ///
    /// The GIL is released while waiting so a Python callable formatter can
    /// run on the worker thread.
    #[pyo3(name = "flush")]
    fn py_flush(&self, py: Python<'_>) -> bool {
        py.detach(|| self.inner.flush())
    }

    /// Close the handler and stop its file worker.
    ///
    /// The GIL is released while waiting so a Python callable formatter can
    /// finish on the worker thread.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// handler.py_close();
    /// ```
    #[pyo3(name = "close")]
    fn py_close(&mut self, py: Python<'_>) {
        py.detach(|| self.inner.close());
    }
}

/// Force upcoming file-reopen attempts to fail so retry handling can be tested.
///
/// The supplied reason is attached to each induced failure. When it is `None`,
/// the test helper uses a default reason.
///
/// # Examples
///
/// ```ignore
/// force_rotating_fresh_failure_for_test(2, Some("simulated reopen failure"))?;
/// ```
#[pyfunction]
pub fn force_rotating_fresh_failure_for_test(count: usize, reason: Option<&str>) -> PyResult<()> {
    let reason = reason
        .map(|value| value.to_string())
        .unwrap_or_else(|| "python requested failure".to_string());
    fresh_failure::set_forced_fresh_failure(count, reason);
    Ok(())
}

/// Clear any forced reopen failures installed by a test.
///
/// # Examples
///
/// ```ignore
/// clear_rotating_fresh_failure_for_test();
/// ```
#[pyfunction]
pub fn clear_rotating_fresh_failure_for_test() {
    fresh_failure::clear_forced_fresh_failure();
}
