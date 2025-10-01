//! Rotating file handler delegating to the file handler implementation.
//!
//! The struct stores rotation thresholds so future updates can implement the
//! actual rollover logic without changing the builder interface.

use std::{
    any::Any,
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use delegate::delegate;

#[cfg(feature = "python")]
use pyo3::prelude::*;

use crate::{
    formatter::FemtoFormatter,
    handler::FemtoHandlerTrait,
    handlers::file::{
        BuilderOptions, FemtoFileHandler, HandlerConfig, RotationStrategy, TestConfig,
    },
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

struct FileRotationStrategy {
    path: PathBuf,
    max_bytes: u64,
    backup_count: usize,
}

impl FileRotationStrategy {
    fn new(path: PathBuf, max_bytes: u64, backup_count: usize) -> Self {
        Self {
            path,
            max_bytes,
            backup_count,
        }
    }

    fn next_record_bytes(message: &str) -> u64 {
        message.len() as u64 + 1
    }

    fn should_rotate(&self, writer: &BufWriter<File>, next_record_bytes: u64) -> io::Result<bool> {
        if self.max_bytes == 0 {
            return Ok(false);
        }
        let current_file_len = writer.get_ref().metadata()?.len();
        let buffered_bytes = writer.buffer().len() as u64;
        Ok(current_file_len + buffered_bytes + next_record_bytes > self.max_bytes)
    }

    fn rotate(&mut self, writer: &mut BufWriter<File>) -> io::Result<()> {
        writer.flush()?;
        if self.backup_count == 0 {
            let file = writer.get_mut();
            file.set_len(0)?;
            file.seek(SeekFrom::Start(0))?;
            return Ok(());
        }
        self.rotate_backups()?;
        if self.path.exists() {
            fs::copy(&self.path, self.backup_path(1))?;
        }
        let file = writer.get_mut();
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        Ok(())
    }

    fn remove_file_if_exists(path: &Path) -> io::Result<()> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err),
        }
    }

    fn rename_file_if_exists(src: &Path, dst: &Path) -> io::Result<()> {
        match fs::rename(src, dst) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err),
        }
    }

    fn remove_excess_backups(&self) -> io::Result<()> {
        let mut extra = self.backup_count + 1;
        loop {
            let candidate = self.backup_path(extra);
            match fs::remove_file(&candidate) {
                Ok(()) => {
                    extra += 1;
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    break;
                }
                Err(err) => {
                    return Err(err);
                }
            }
        }
        Ok(())
    }

    fn cascade_backups(&self) -> io::Result<()> {
        for idx in (1..self.backup_count).rev() {
            let src = self.backup_path(idx);
            if src.exists() {
                let dst = self.backup_path(idx + 1);
                Self::rename_file_if_exists(&src, &dst)?;
            }
        }
        Ok(())
    }

    fn rotate_backups(&self) -> io::Result<()> {
        if self.backup_count == 0 {
            return Ok(());
        }
        self.remove_excess_backups()?;
        let oldest = self.backup_path(self.backup_count);
        Self::remove_file_if_exists(&oldest)?;
        self.cascade_backups()?;
        Ok(())
    }

    fn backup_path(&self, index: usize) -> PathBuf {
        let mut backup = self.path.clone();
        let mut name = self
            .path
            .file_name()
            .map(|file_name| file_name.to_os_string())
            .unwrap_or_else(|| self.path.as_os_str().to_os_string());
        name.push(format!(".{index}"));
        backup.set_file_name(name);
        backup
    }
}

impl RotationStrategy<BufWriter<File>> for FileRotationStrategy {
    fn before_write(&mut self, writer: &mut BufWriter<File>, formatted: &str) -> io::Result<()> {
        let next_bytes = Self::next_record_bytes(formatted);
        if self.should_rotate(writer, next_bytes)? {
            self.rotate(writer)?;
        }
        Ok(())
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
        let rotation = if rotation_config.max_bytes == 0 {
            None
        } else {
            Some(FileRotationStrategy::new(
                path_ref.to_path_buf(),
                rotation_config.max_bytes,
                rotation_config.backup_count,
            ))
        };
        let options = BuilderOptions::<BufWriter<File>, FileRotationStrategy>::new(rotation, None);
        let handler = FemtoFileHandler::build_from_worker(writer, formatter, config, options);
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
        let overflow_policy = file::policy::parse_policy_string(&policy)?;
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

#[cfg(test)]
mod tests;
