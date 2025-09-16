//! Rotating file handler delegating to the file handler implementation.
//!
//! The struct stores rotation thresholds so future updates can implement the
//! actual rollover logic without changing the builder interface.

use std::{any::Any, io, path::Path};

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
    /// Construct a handler from its constituent parts.
    pub(crate) fn from_parts(inner: FemtoFileHandler, max_bytes: u64, backup_count: usize) -> Self {
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
        max_bytes: u64,
        backup_count: usize,
    ) -> io::Result<Self>
    where
        P: AsRef<Path>,
        F: FemtoFormatter + Send + 'static,
    {
        let inner = FemtoFileHandler::with_capacity_flush_policy(path, formatter, config)?;
        Ok(Self::from_parts(inner, max_bytes, backup_count))
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
        Self::from_parts(inner, max_bytes, backup_count)
    }

    /// Flush any queued log records.
    pub fn flush(&self) -> bool {
        self.inner.flush()
    }

    /// Close the handler, waiting for the worker thread to shut down.
    pub fn close(&mut self) {
        self.inner.close();
    }
}

#[cfg(feature = "python")]
#[pymethods]
impl FemtoRotatingFileHandler {
    #[new]
    #[pyo3(signature = (
        path,
        max_bytes = 0,
        backup_count = 0,
        capacity = DEFAULT_CHANNEL_CAPACITY,
        flush_interval = 1,
        policy = "drop"
    ))]
    fn py_new(
        path: String,
        max_bytes: u64,
        backup_count: usize,
        capacity: usize,
        flush_interval: isize,
        policy: &str,
    ) -> PyResult<Self> {
        let overflow_policy = file::parse_overflow_policy(policy)?;
        let flush_interval = file::validate_params(capacity, flush_interval)?;
        let handler_cfg = HandlerConfig {
            capacity,
            flush_interval,
            overflow_policy,
        };
        let inner =
            FemtoFileHandler::with_capacity_flush_policy(&path, DefaultFormatter, handler_cfg)
                .map_err(|err| pyo3::exceptions::PyIOError::new_err(format!("{path}: {err}")))?;
        Ok(Self::from_parts(inner, max_bytes, backup_count))
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
        <Self as FemtoHandlerTrait>::handle(self, FemtoLogRecord::new(logger, level, message));
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
    fn handle(&self, record: FemtoLogRecord) {
        FemtoHandlerTrait::handle(&self.inner, record);
    }

    fn flush(&self) -> bool {
        self.inner.flush()
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
