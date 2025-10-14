//! Public API for the file-based logging handler.
//!
//! `FemtoFileHandler` spawns a dedicated worker thread that writes formatted
//! log records to disk. Configuration types and the worker implementation live
//! in submodules and are re-exported here for external use.
//!
//! Construct the handler with [`FemtoFileHandler::new`] for defaults,
//! [`FemtoFileHandler::with_capacity`] to tune the queue size, or
//! [`FemtoFileHandler::with_capacity_flush_policy`] for full control in Rust.
//! Python callers customise these options via keyword arguments to
//! `FemtoFileHandler`.
//!
//! The flush interval must be greater than zero. A value of 1 flushes on every
//! record.

mod builder_options;
mod config;
mod drop_warner;
pub(crate) mod policy;
mod worker;
pub(crate) use worker::{NoRotation, RotationStrategy};
#[cfg(test)]
pub(crate) mod test_support;

pub(crate) use builder_options::BuilderOptions;

use drop_warner::{DropReason, DropWarner};

use std::{
    fs::{File, OpenOptions},
    io::{self, BufWriter, Seek, Write},
    path::Path,
    thread::JoinHandle,
    time::Duration,
};

use crossbeam_channel::{Receiver, SendTimeoutError, Sender, TrySendError};
use pyo3::prelude::*;
use std::any::Any;

use crate::handler::{to_py_runtime_error, FemtoHandlerTrait, HandlerError};
use crate::{
    formatter::{DefaultFormatter, FemtoFormatter},
    log_record::FemtoLogRecord,
};

pub use config::{HandlerConfig, OverflowPolicy, TestConfig, DEFAULT_CHANNEL_CAPACITY};
use worker::{spawn_worker, FileCommand, WorkerConfig};

/// Internal items needed by the worker implementation.
mod mod_impl {
    use super::*;

    pub(super) fn write_record<W>(
        writer: &mut W,
        message: &str,
        flush_tracker: &mut worker::FlushTracker,
    ) -> io::Result<()>
    where
        W: Write,
    {
        writeln!(writer, "{message}")?;
        flush_tracker.record_write(writer)
    }
}

/// File-based logging handler exposed to Python.
///
/// Spawns a worker thread that writes formatted records to disk using the
/// configuration provided at construction time.
#[pyclass]
pub struct FemtoFileHandler {
    tx: Option<Sender<FileCommand>>,
    handle: Option<JoinHandle<()>>,
    done_rx: Receiver<()>,
    overflow_policy: OverflowPolicy,
    ack_rx: Receiver<()>,
    drop_warner: DropWarner,
}

fn open_log_file(path: &str) -> PyResult<File> {
    use pyo3::exceptions::PyIOError;
    #[expect(
        clippy::ineffective_open_options,
        reason = "Be explicit about write intent alongside append"
    )]
    OpenOptions::new()
        .create(true)
        .write(true)
        .append(true)
        .open(path)
        .map_err(|e| PyIOError::new_err(format!("{path}: {e}")))
}

pub(crate) fn validate_params(capacity: usize, flush_interval: isize) -> PyResult<usize> {
    use pyo3::exceptions::PyValueError;
    if capacity == 0 {
        return Err(PyValueError::new_err("capacity must be greater than zero"));
    }
    if flush_interval <= 0 {
        return Err(PyValueError::new_err(
            "flush_interval must be greater than zero",
        ));
    }
    Ok(flush_interval as usize)
}

#[pymethods]
impl FemtoFileHandler {
    /// Create a file handler writing to `path`.
    ///
    /// Python usage:
    ///   `FemtoFileHandler(path, capacity=DEFAULT_CHANNEL_CAPACITY,`
    ///   `flush_interval=1, policy="drop")`
    ///
    /// - `capacity` must be greater than zero.
    /// - `flush_interval` must be greater than zero.
    /// - `policy` is one of: `"drop"`, `"block"`, or `"timeout:N"` (N > 0).
    #[new]
    #[pyo3(
        text_signature = "(path, capacity=DEFAULT_CHANNEL_CAPACITY, flush_interval=1, policy='drop')"
    )]
    #[pyo3(signature=(
        path,
        capacity = DEFAULT_CHANNEL_CAPACITY,
        flush_interval = 1,
        policy = "drop"
    ))]
    fn py_new(
        path: String,
        capacity: usize,
        flush_interval: isize,
        policy: &str,
    ) -> PyResult<Self> {
        let overflow_policy = policy::parse_policy_string(policy)
            .map_err(|err| pyo3::exceptions::PyValueError::new_err(err.to_string()))?;
        let flush_interval = validate_params(capacity, flush_interval)?;
        let handler_cfg = HandlerConfig {
            capacity,
            flush_interval,
            overflow_policy,
        };
        let file = open_log_file(&path)?;
        Ok(FemtoFileHandler::from_file(
            file,
            DefaultFormatter,
            handler_cfg,
        ))
    }

    #[pyo3(name = "handle")]
    fn py_handle(&self, logger: &str, level: &str, message: &str) -> PyResult<()> {
        <Self as FemtoHandlerTrait>::handle(self, FemtoLogRecord::new(logger, level, message))
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

impl FemtoFileHandler {
    /// Create a handler writing to `path` with default settings.
    pub fn new<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        Self::with_capacity(path, DefaultFormatter, DEFAULT_CHANNEL_CAPACITY)
    }

    /// Create a handler with a custom queue `capacity` and default drop policy.
    ///
    /// The handler flushes the file after every record (`flush_interval = 1`).
    pub fn with_capacity<P, F>(path: P, formatter: F, capacity: usize) -> io::Result<Self>
    where
        P: AsRef<Path>,
        F: FemtoFormatter + Send + 'static,
    {
        let cfg = HandlerConfig {
            capacity,
            flush_interval: 1,
            overflow_policy: OverflowPolicy::Drop,
        };
        Self::with_capacity_flush_policy(path, formatter, cfg)
    }

    /// Create a handler using an explicit [`HandlerConfig`].
    ///
    /// This allows callers to override the queue capacity, flush interval (> 0)
    /// and overflow policy in a single place.
    pub fn with_capacity_flush_policy<P, F>(
        path: P,
        formatter: F,
        config: HandlerConfig,
    ) -> io::Result<Self>
    where
        P: AsRef<Path>,
        F: FemtoFormatter + Send + 'static,
    {
        if config.flush_interval == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "flush_interval must be greater than zero",
            ));
        }
        #[expect(
            clippy::ineffective_open_options,
            reason = "Be explicit about write intent alongside append"
        )]
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(true)
            .open(path)?;
        Ok(Self::from_file(file, formatter, config))
    }

    fn from_file<F>(file: File, formatter: F, config: HandlerConfig) -> Self
    where
        F: FemtoFormatter + Send + 'static,
    {
        // Use a buffered writer so flush policies control when records are
        // persisted to disk. Without buffering each write reaches the OS
        // immediately, causing premature flushes and defeating the configured
        // `flush_interval`.
        let writer = BufWriter::new(file);
        Self::build_from_worker(
            writer,
            formatter,
            config,
            BuilderOptions::<BufWriter<File>>::default(),
        )
    }

    pub fn flush(&self) -> bool {
        self.drop_warner.flush();
        match &self.tx {
            Some(tx) => self.perform_flush(tx),
            None => false,
        }
    }

    fn perform_flush(&self, tx: &Sender<FileCommand>) -> bool {
        if tx.send(FileCommand::Flush).is_err() {
            return false;
        }
        self.wait_for_flush_completion()
    }

    fn wait_for_flush_completion(&self) -> bool {
        self.ack_rx.recv_timeout(Duration::from_secs(1)).is_ok()
    }

    fn record_drop(&self, reason: DropReason, error: HandlerError) -> HandlerError {
        self.drop_warner.record(reason);
        error
    }

    pub fn close(&mut self) {
        self.tx.take();
        self.drop_warner.flush();
        if let Some(handle) = self.handle.take() {
            if self.done_rx.recv_timeout(Duration::from_secs(1)).is_err() {
                log::warn!("FemtoFileHandler: worker thread did not shut down within 1s");
                return;
            }
            if handle.join().is_err() {
                log::warn!("FemtoFileHandler: worker thread panicked");
            }
        }
    }

    pub(crate) fn build_from_worker<W, F, R>(
        writer: W,
        formatter: F,
        config: HandlerConfig,
        options: BuilderOptions<W, R>,
    ) -> Self
    where
        W: Write + Seek + Send + 'static,
        F: FemtoFormatter + Send + 'static,
        R: RotationStrategy<W> + Send + 'static,
    {
        let BuilderOptions {
            rotation,
            start_barrier,
            ..
        } = options;
        let mut worker_cfg = WorkerConfig::from(&config);
        worker_cfg.start_barrier = start_barrier;
        let overflow_policy = config.overflow_policy;
        let (ack_tx, ack_rx) = crossbeam_channel::unbounded();
        let (tx, done_rx, handle) = spawn_worker(writer, formatter, worker_cfg, ack_tx, rotation);
        let drop_warner = DropWarner::new(overflow_policy);
        Self {
            tx: Some(tx),
            handle: Some(handle),
            done_rx,
            overflow_policy,
            ack_rx,
            drop_warner,
        }
    }

    pub fn with_writer_for_test<W, F>(config: TestConfig<W, F>) -> Self
    where
        W: Write + Seek + Send + 'static,
        F: FemtoFormatter + Send + 'static,
    {
        let TestConfig {
            writer,
            formatter,
            capacity,
            flush_interval,
            overflow_policy,
            start_barrier,
        } = config;
        let handler_config = HandlerConfig {
            capacity,
            flush_interval,
            overflow_policy,
        };
        let options = BuilderOptions::<W>::new(NoRotation, start_barrier);
        Self::build_from_worker(writer, formatter, handler_config, options)
    }
}

impl FemtoHandlerTrait for FemtoFileHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        let Some(tx) = &self.tx else {
            return Err(self.record_drop(DropReason::Closed, HandlerError::Closed));
        };
        match self.overflow_policy {
            OverflowPolicy::Drop => match tx.try_send(FileCommand::Record(Box::new(record))) {
                Ok(()) => Ok(()),
                Err(TrySendError::Full(_)) => {
                    Err(self.record_drop(DropReason::QueueFull, HandlerError::QueueFull))
                }
                Err(TrySendError::Disconnected(_)) => {
                    Err(self.record_drop(DropReason::Closed, HandlerError::Closed))
                }
            },
            OverflowPolicy::Block => match tx.send(FileCommand::Record(Box::new(record))) {
                Ok(()) => Ok(()),
                Err(_) => Err(self.record_drop(DropReason::Closed, HandlerError::Closed)),
            },
            OverflowPolicy::Timeout(dur) => {
                match tx.send_timeout(FileCommand::Record(Box::new(record)), dur) {
                    Ok(()) => Ok(()),
                    Err(SendTimeoutError::Timeout(_)) => {
                        Err(self.record_drop(DropReason::Timeout, HandlerError::Timeout(dur)))
                    }
                    Err(SendTimeoutError::Disconnected(_)) => {
                        Err(self.record_drop(DropReason::Closed, HandlerError::Closed))
                    }
                }
            }
        }
    }

    fn flush(&self) -> bool {
        FemtoFileHandler::flush(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Drop for FemtoFileHandler {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests;
