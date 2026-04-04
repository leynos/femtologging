//! Public API for the file-based logging handler.
//!
//! `FemtoFileHandler` spawns a dedicated worker thread that writes formatted
//! log records to disk. Configuration types and the worker implementation live
//! in submodules and are re-exported here for external use.
//!
//! Construct the handler with [`FemtoFileHandler::new`] for defaults,
//! [`FemtoFileHandler::with_capacity`] to tune the queue size, or
//! [`FemtoFileHandler::with_capacity_flush_policy`] for full control in Rust.
//! Python callers customize these options via keyword arguments to
//! `FemtoFileHandler`.
//!
//! The flush interval must be greater than zero. A value of 1 flushes on every
//! record.
mod builder_options;
mod config;
mod handler_impl;
mod io_utils;
pub(crate) mod policy;
mod validations;
mod worker;
pub(crate) use worker::{NoRotation, RotationStrategy};
#[cfg(test)]
pub(crate) mod test_support;

use std::{
    fs::File,
    io::{self, BufWriter, Seek, Write},
    path::Path,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, Sender};
use pyo3::prelude::*;

use crate::handler::FemtoHandlerTrait;
#[cfg(test)]
use crate::level::FemtoLevel;
use crate::{
    formatter::{DefaultFormatter, FemtoFormatter},
    log_record::FemtoLogRecord,
};

pub(crate) use builder_options::BuilderOptions;
pub use config::{DEFAULT_CHANNEL_CAPACITY, HandlerConfig, OverflowPolicy, TestConfig};
use io_utils::open_log_file;
pub(crate) use validations::{
    validate_capacity_nonzero, validate_flush_interval_nonzero, validate_params,
};
use worker::{FileCommand, WorkerConfig, spawn_worker};

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
        capacity = DEFAULT_CHANNEL_CAPACITY as isize,
        flush_interval = 1,
        policy = "drop"
    ))]
    fn py_new(
        path: String,
        capacity: isize,
        flush_interval: isize,
        policy: &str,
    ) -> PyResult<Self> {
        let overflow_policy = policy::parse_policy_string(policy)
            .map_err(|err| pyo3::exceptions::PyValueError::new_err(err.to_string()))?;
        let (capacity, flush_interval) = validate_params(capacity, flush_interval)?;
        let handler_cfg = HandlerConfig {
            capacity,
            flush_interval,
            overflow_policy,
        };
        let file = open_log_file(&path)
            .map_err(|err| pyo3::exceptions::PyIOError::new_err(err.to_string()))?;
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
    /// This allows callers to override the queue capacity (> 0), flush interval
    /// (> 0), and overflow policy in a single place.
    pub fn with_capacity_flush_policy<P, F>(
        path: P,
        formatter: F,
        config: HandlerConfig,
    ) -> io::Result<Self>
    where
        P: AsRef<Path>,
        F: FemtoFormatter + Send + 'static,
    {
        validate_capacity_nonzero(config.capacity)
            .map_err(|msg| io::Error::new(io::ErrorKind::InvalidInput, msg))?;
        validate_flush_interval_nonzero(config.flush_interval)
            .map_err(|msg| io::Error::new(io::ErrorKind::InvalidInput, msg))?;
        let file = open_log_file(path)?;
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
        let deadline = Instant::now() + Duration::from_secs(1);
        let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
        let remaining = deadline.saturating_duration_since(Instant::now());
        if tx
            .send_timeout(FileCommand::Flush(ack_tx), remaining)
            .is_err()
        {
            return false;
        }
        self.wait_for_flush_completion(&ack_rx, deadline)
    }

    fn wait_for_flush_completion(
        &self,
        ack_rx: &Receiver<io::Result<()>>,
        deadline: Instant,
    ) -> bool {
        let remaining = deadline.saturating_duration_since(Instant::now());
        matches!(ack_rx.recv_timeout(remaining), Ok(Ok(())))
    }

    /// Close the handler and wait for the worker to stop.
    ///
    /// This method is idempotent. Calling it multiple times is safe; only the
    /// first call performs shutdown work.
    ///
    /// Threading
    /// ---------
    /// The method requires `&mut self`, so callers must ensure exclusive access
    /// when invoking it. Concurrent calls from multiple threads must be
    /// synchronized externally (for example, with a `Mutex`).
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
        let (tx, done_rx, handle) = spawn_worker(writer, formatter, worker_cfg, rotation);
        Self {
            tx: Some(tx),
            handle: Some(handle),
            done_rx,
            overflow_policy,
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

#[cfg(test)]
mod flush_ack_tests;
#[cfg(test)]
mod tests;
