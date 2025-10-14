//! Rotating file handler delegating to the file handler implementation.
//!
//! The struct stores rotation thresholds so future updates can implement the
//! actual rollover logic without changing the builder interface.

use std::{
    any::Any,
    fs::{File, OpenOptions},
    io::{self, BufWriter},
    path::Path,
};

use delegate::delegate;

#[cfg(feature = "python")]
use pyo3::prelude::*;

use crate::{
    formatter::FemtoFormatter,
    handler::{FemtoHandlerTrait, HandlerError},
    handlers::file::{BuilderOptions, FemtoFileHandler, HandlerConfig, NoRotation, TestConfig},
    log_record::FemtoLogRecord,
};

#[cfg(feature = "python")]
use crate::handler::to_py_runtime_error;

mod fresh_failure;
mod strategy;
pub(crate) use strategy::FileRotationStrategy;

#[cfg(test)]
pub(crate) use fresh_failure::force_fresh_failure_once_for_test;

#[cfg(feature = "python")]
use crate::{
    formatter::DefaultFormatter,
    handlers::file::{self, DEFAULT_CHANNEL_CAPACITY},
};

#[cfg(feature = "python")]
#[pyfunction]
pub(crate) fn force_rotating_fresh_failure_for_test(
    count: usize,
    reason: Option<&str>,
) -> PyResult<()> {
    let reason = reason
        .map(|value| value.to_string())
        .unwrap_or_else(|| "python requested failure".to_string());
    fresh_failure::set_forced_fresh_failure(count, reason);
    Ok(())
}

#[cfg(feature = "python")]
#[pyfunction]
pub(crate) fn clear_rotating_fresh_failure_for_test() {
    fresh_failure::clear_forced_fresh_failure();
}

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
///     Some((1024, 3)),
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

#[cfg(feature = "python")]
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
        let path_ref = path.as_ref();
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path_ref)?;
        let writer = BufWriter::new(file);
        let handler = if rotation_config.max_bytes == 0 {
            let options = BuilderOptions::<BufWriter<File>>::new(NoRotation, None);
            FemtoFileHandler::build_from_worker(writer, formatter, config, options)
        } else {
            let rotation = FileRotationStrategy::new(
                path_ref.to_path_buf(),
                rotation_config.max_bytes,
                rotation_config.backup_count,
            );
            let options = BuilderOptions::<BufWriter<File>, _>::new(rotation, None);
            FemtoFileHandler::build_from_worker(writer, formatter, config, options)
        };
        Ok(Self::new_with_rotation_limits(
            handler,
            rotation_config.max_bytes,
            rotation_config.backup_count,
        ))
    }

    /// Build a handler for tests using the in-memory writer helper.
    pub fn with_writer_for_test<W, F>(
        config: TestConfig<W, F>,
        max_bytes: u64,
        backup_count: usize,
    ) -> Self
    where
        W: std::io::Write + std::io::Seek + Send + 'static,
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
        self.max_bytes
    }

    /// Expose the configured backup count.
    #[getter]
    fn backup_count(&self) -> usize {
        self.backup_count
    }

    #[pyo3(name = "handle")]
    fn py_handle(&self, logger: &str, level: &str, message: &str) -> PyResult<()> {
        self.inner
            .handle(FemtoLogRecord::new(logger, level, message))
            .map_err(|err| to_py_runtime_error(&err))
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
            fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError>;
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

#[cfg(test)]
mod tests;
