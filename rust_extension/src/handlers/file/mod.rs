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
mod config;
pub(crate) mod policy;
mod worker;
pub(crate) use worker::{NoRotation, RotationStrategy};
#[cfg(test)]
pub(crate) mod test_support;

use std::{
    fs::{File, OpenOptions},
    io::{self, BufWriter, Seek, Write},
    marker::PhantomData,
    path::Path,
    sync::{Arc, Barrier},
    thread::JoinHandle,
    time::Duration,
};

use crossbeam_channel::{Receiver, SendTimeoutError, Sender, TrySendError};
use pyo3::prelude::*;
use std::any::Any;

use crate::handler::{FemtoHandlerTrait, HandlerError};
#[cfg(test)]
use crate::level::FemtoLevel;
use crate::{
    formatter::{DefaultFormatter, FemtoFormatter},
    log_record::FemtoLogRecord,
};

pub use config::{DEFAULT_CHANNEL_CAPACITY, HandlerConfig, OverflowPolicy, TestConfig};
use worker::{FileCommand, WorkerConfig, spawn_worker};

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
        let parsed_level = crate::level::FemtoLevel::parse_py(level)?;
        <Self as FemtoHandlerTrait>::handle(
            self,
            FemtoLogRecord::new(logger, parsed_level, message),
        )
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("Handler error: {e}")))
    }

    /// Flush queued log records to the underlying file without closing the
    /// handler.
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

pub(crate) struct BuilderOptions<W, R = NoRotation>
where
    W: Write + Seek,
    R: RotationStrategy<W>,
{
    pub(crate) rotation: R,
    pub(crate) start_barrier: Option<Arc<Barrier>>,
    _phantom: PhantomData<W>,
}

impl<W> Default for BuilderOptions<W>
where
    W: Write + Seek,
{
    fn default() -> Self {
        Self {
            rotation: NoRotation,
            start_barrier: None,
            _phantom: PhantomData,
        }
    }
}

impl<W, R> BuilderOptions<W, R>
where
    W: Write + Seek,
    R: RotationStrategy<W>,
{
    pub(crate) fn new(rotation: R, start_barrier: Option<Arc<Barrier>>) -> Self {
        Self {
            rotation,
            start_barrier,
            _phantom: PhantomData,
        }
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

    pub fn close(&mut self) {
        self.tx.take();
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
        let (tx, done_rx, ack_rx, handle) = spawn_worker(writer, formatter, worker_cfg, rotation);
        Self {
            tx: Some(tx),
            handle: Some(handle),
            done_rx,
            overflow_policy,
            ack_rx,
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
            log::warn!("FemtoFileHandler: handle called after close");
            return Err(HandlerError::Closed);
        };
        match self.overflow_policy {
            OverflowPolicy::Drop => match tx.try_send(FileCommand::Record(Box::new(record))) {
                Ok(()) => Ok(()),
                Err(TrySendError::Full(_)) => {
                    log::warn!(
                        "FemtoFileHandler (Drop): queue full or shutting down, dropping record"
                    );
                    Err(HandlerError::QueueFull)
                }
                Err(TrySendError::Disconnected(_)) => {
                    log::warn!("FemtoFileHandler (Drop): queue closed, dropping record");
                    Err(HandlerError::Closed)
                }
            },
            OverflowPolicy::Block => match tx.send(FileCommand::Record(Box::new(record))) {
                Ok(()) => Ok(()),
                Err(_) => {
                    log::warn!(
                        "FemtoFileHandler (Block): queue full or shutting down, dropping record"
                    );
                    Err(HandlerError::Closed)
                }
            },
            OverflowPolicy::Timeout(dur) => {
                match tx.send_timeout(FileCommand::Record(Box::new(record)), dur) {
                    Ok(()) => Ok(()),
                    Err(SendTimeoutError::Timeout(_)) => {
                        log::warn!(
                            "FemtoFileHandler (Timeout): timed out waiting for queue, dropping record"
                        );
                        Err(HandlerError::Timeout(dur))
                    }
                    Err(SendTimeoutError::Disconnected(_)) => {
                        log::warn!("FemtoFileHandler (Timeout): queue closed, dropping record");
                        Err(HandlerError::Closed)
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
