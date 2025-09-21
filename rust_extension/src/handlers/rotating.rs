//! Rotating file handler delegating to the file handler implementation.
//!
//! The struct stores rotation thresholds so future updates can implement the
//! actual rollover logic without changing the builder interface.

use std::{any::Any, io, path::Path};

use delegate::delegate;

#[cfg(feature = "python")]
use pyo3::prelude::*;

use crate::{
    formatter::FemtoFormatter,
    handler::FemtoHandlerTrait,
    handlers::file::{FemtoFileHandler, HandlerConfig, TestConfig},
    log_record::FemtoLogRecord,
};

#[cfg(feature = "python")]
use crate::{
    formatter::DefaultFormatter,
    handlers::file::{self, DEFAULT_CHANNEL_CAPACITY},
};

/// Rotation thresholds controlling when a file rolls over.
///
/// Grouping the limits together keeps the handler constructor concise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RotationConfig {
    pub max_bytes: u64,
    pub backup_count: usize,
}

impl RotationConfig {
    /// Create a rotation configuration with explicit limits.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let config = RotationConfig::new(1024, 3);
    /// assert_eq!(config.max_bytes, 1024);
    /// assert_eq!(config.backup_count, 3);
    /// ```
    pub const fn new(max_bytes: u64, backup_count: usize) -> Self {
        Self {
            max_bytes,
            backup_count,
        }
    }

    /// Return a configuration that disables rotation.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let config = RotationConfig::disabled();
    /// assert_eq!(config.max_bytes, 0);
    /// assert_eq!(config.backup_count, 0);
    /// ```
    pub const fn disabled() -> Self {
        Self::new(0, 0)
    }
}

impl Default for RotationConfig {
    fn default() -> Self {
        Self::disabled()
    }
}

#[cfg(feature = "python")]
/// Error message describing how to configure rotation thresholds.
pub const ROTATION_VALIDATION_MSG: &str =
    "both max_bytes and backup_count must be > 0 to enable rotation; set both to 0 to disable";

/// Python options bundling queue and rotation configuration for rotating
/// file handlers during instantiation.
///
/// The options map onto the capacity, flushing, overflow policy, and rotation
/// thresholds exposed by [`FemtoFileHandler`] and default to the existing
/// values to preserve backwards compatibility.
///
/// # Examples
///
/// ```ignore
/// let options = HandlerOptions::new(
///     64,
///     2,
///     "drop".to_string(),
///     1024,
///     3,
/// )
/// .expect("valid options");
/// assert_eq!(options.capacity, 64);
/// assert_eq!(options.flush_interval, 2);
/// assert_eq!(options.policy, "drop");
/// assert_eq!(options.max_bytes, 1024);
/// assert_eq!(options.backup_count, 3);
/// ```
#[cfg(feature = "python")]
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

#[cfg(feature = "python")]
#[pymethods]
impl HandlerOptions {
    #[new]
    #[pyo3(
        text_signature = "(capacity=DEFAULT_CHANNEL_CAPACITY, flush_interval=1, policy='drop', max_bytes=0, backup_count=0)"
    )]
    #[pyo3(signature = (
        capacity = DEFAULT_CHANNEL_CAPACITY,
        flush_interval = 1,
        policy = "drop".to_string(),
        max_bytes = 0,
        backup_count = 0,
    ))]
    fn new(
        capacity: usize,
        flush_interval: isize,
        policy: String,
        max_bytes: u64,
        backup_count: usize,
    ) -> PyResult<Self> {
        if flush_interval == -1 {
            file::validate_params(capacity, 1)?;
        } else {
            file::validate_params(capacity, flush_interval)?;
        }
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

#[cfg(feature = "python")]
impl Default for HandlerOptions {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_CHANNEL_CAPACITY,
            flush_interval: -1,
            policy: "drop".to_string(),
            max_bytes: 0,
            backup_count: 0,
        }
    }
}

/// File handler variant configured for size-based rotation.
///
/// The handler currently delegates all I/O to [`FemtoFileHandler`], recording
/// rotation thresholds so later work can implement the rollover behaviour.
#[cfg_attr(feature = "python", pyclass)]
pub struct FemtoRotatingFileHandler {
    inner: FemtoFileHandler,
    max_bytes: u64,
    backup_count: usize,
}

impl FemtoRotatingFileHandler {
    /// Construct a handler by pairing a file handler with rotation limits.
    ///
    /// Internal visibility allows the builder to construct instances whilst
    /// preventing external crates from bypassing validation.
    pub(crate) fn new_with_rotation_limits(
        inner: FemtoFileHandler,
        max_bytes: u64,
        backup_count: usize,
    ) -> Self {
        Self {
            inner,
            max_bytes,
            backup_count,
        }
    }

    /// Return the configured rotation thresholds.
    pub(crate) fn rotation_limits(&self) -> (u64, usize) {
        (self.max_bytes, self.backup_count)
    }

    /// Build a rotating handler with the supplied configuration.
    pub fn with_capacity_flush_policy<P, F>(
        path: P,
        formatter: F,
        config: HandlerConfig,
        rotation_config: RotationConfig,
    ) -> io::Result<Self>
    where
        P: AsRef<Path>,
        F: FemtoFormatter + Send + 'static,
    {
        let inner = FemtoFileHandler::with_capacity_flush_policy(path, formatter, config)?;
        let RotationConfig {
            max_bytes,
            backup_count,
        } = rotation_config;
        Ok(Self::new_with_rotation_limits(
            inner,
            max_bytes,
            backup_count,
        ))
    }

    /// Build a handler for tests using the in-memory writer helper.
    pub fn with_writer_for_test<W, F>(
        config: TestConfig<W, F>,
        max_bytes: u64,
        backup_count: usize,
    ) -> Self
    where
        W: std::io::Write + Send + 'static,
        F: FemtoFormatter + Send + 'static,
    {
        let inner = FemtoFileHandler::with_writer_for_test(config);
        Self::new_with_rotation_limits(inner, max_bytes, backup_count)
    }

    delegate! {
        to self.inner {
            /// Flush any queued log records.
            pub fn flush(&self) -> bool;
            /// Close the handler, waiting for the worker thread to shut down.
            pub fn close(&mut self);
        }
    }
}

#[cfg(feature = "python")]
#[pymethods]
impl FemtoRotatingFileHandler {
    #[new]
    #[pyo3(text_signature = "(path, options=None)")]
    #[pyo3(signature = (path, options = None))]
    fn py_new(path: String, options: Option<HandlerOptions>) -> PyResult<Self> {
        let provided_options = options.is_some();
        let opts = options.unwrap_or_else(HandlerOptions::default);
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
        let overflow_policy = file::parse_overflow_policy(&policy)?;
        let flush_interval = if provided_options || flush_interval > 0 {
            file::validate_params(capacity, flush_interval)?
        } else {
            file::validate_params(capacity, 1)?
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
        self.max_bytes
    }

    /// Expose the configured backup count.
    #[getter]
    fn backup_count(&self) -> usize {
        self.backup_count
    }

    #[pyo3(name = "handle")]
    fn py_handle(&self, logger: &str, level: &str, message: &str) {
        self.inner
            .handle(FemtoLogRecord::new(logger, level, message));
    }

    #[pyo3(name = "flush")]
    fn py_flush(&self) -> bool {
        self.flush()
    }

    #[pyo3(name = "close")]
    fn py_close(&mut self) {
        self.close();
    }
}

impl FemtoHandlerTrait for FemtoRotatingFileHandler {
    delegate! {
        to self.inner {
            fn handle(&self, record: FemtoLogRecord);
            fn flush(&self) -> bool;
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Drop for FemtoRotatingFileHandler {
    fn drop(&mut self) {
        self.close();
    }
}
