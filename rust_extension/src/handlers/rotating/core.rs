//! Core rotating file handler logic.
//!
//! This module owns rotation configuration and delegates all I/O to
//! [`FemtoFileHandler`], remaining independent from Python bindings.

use std::{
    any::Any,
    fs::{File, OpenOptions},
    io::{self, BufWriter},
    path::Path,
};

use delegate::delegate;

use super::strategy::FileRotationStrategy;
use crate::{
    formatter::FemtoFormatter,
    handler::{FemtoHandlerTrait, HandlerError},
    handlers::file::{BuilderOptions, FemtoFileHandler, HandlerConfig, NoRotation, TestConfig},
    log_record::FemtoLogRecord,
};

/// Rotation thresholds controlling when a file rolls over.
///
/// Grouping the limits together keeps the handler constructor concise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RotationConfig {
    /// Maximum file size in bytes before rollover is triggered.
    ///
    /// Set to `0` to disable rotation.
    pub max_bytes: u64,
    /// Number of rotated backup files to retain.
    ///
    /// Set to `0` to keep no backups.
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

/// File handler variant configured for size-based rotation.
///
/// For file-backed handlers, this wrapper wires [`FileRotationStrategy`] into
/// [`FemtoFileHandler`] so rollover executes on the worker thread. The wrapper
/// owns rotation thresholds and delegates queueing, flushing, and shutdown to
/// [`FemtoFileHandler`].
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
        let (handler, effective_max_bytes, effective_backup_count) =
            if rotation_config.max_bytes == 0 {
                let options = BuilderOptions::<BufWriter<File>>::new(NoRotation, None);
                (
                    FemtoFileHandler::build_from_worker(writer, formatter, config, options),
                    0,
                    0,
                )
            } else {
                let rotation = FileRotationStrategy::new(
                    path_ref.to_path_buf(),
                    rotation_config.max_bytes,
                    rotation_config.backup_count,
                );
                let options = BuilderOptions::<BufWriter<File>, _>::new(rotation, None);
                (
                    FemtoFileHandler::build_from_worker(writer, formatter, config, options),
                    rotation_config.max_bytes,
                    rotation_config.backup_count,
                )
            };
        Ok(Self::new_with_rotation_limits(
            handler,
            effective_max_bytes,
            effective_backup_count,
        ))
    }

    /// Build a handler for tests using the generic writer helper.
    ///
    /// This delegates to [`FemtoFileHandler::with_writer_for_test`], which
    /// accepts arbitrary writers and therefore does not attach
    /// [`FileRotationStrategy`]. Rotation is disabled for this constructor.
    pub fn with_writer_for_test<W, F>(config: TestConfig<W, F>) -> Self
    where
        W: std::io::Write + std::io::Seek + Send + 'static,
        F: FemtoFormatter + Send + 'static,
    {
        let inner = FemtoFileHandler::with_writer_for_test(config);
        let disabled = RotationConfig::disabled();
        Self::new_with_rotation_limits(inner, disabled.max_bytes, disabled.backup_count)
    }

    delegate! {
        to self.inner {
            /// Flush any queued log records.
            pub fn flush(&self) -> bool;
            /// Close the handler, waiting for the worker thread to shut down.
            ///
            /// This method is idempotent. Calling it multiple times is safe;
            /// only the first call performs shutdown work.
            ///
            /// The method requires `&mut self`, so callers must ensure
            /// exclusive access when invoking it. Concurrent calls from
            /// multiple threads must be synchronized externally (for example,
            /// with a `Mutex`).
            pub fn close(&mut self);
        }
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
