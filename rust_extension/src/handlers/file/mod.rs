//! Public API for the file-based logging handler.
//!
//! `FemtoFileHandler` spawns a dedicated worker thread that writes formatted
//! log records to disk. Configuration types and the worker implementation live
//! in submodules and are re-exported here for external use.
//!
//! Construct the handler with [`new`] for defaults, [`with_capacity`] to tune
//! the queue size, or [`with_capacity_flush_policy`] for full control in Rust.
//! Python callers customise these options via keyword arguments to
//! ``FemtoFileHandler``.
//!
//! The flush interval must be greater than zero. A value of 1 flushes on every
//! record.

mod config;
mod worker;

use std::{
    fs::{File, OpenOptions},
    io::{self, BufWriter, Write},
    path::Path,
    thread::JoinHandle,
    time::Duration,
};

use crossbeam_channel::{Receiver, Sender};
use pyo3::prelude::*;
use std::any::Any;

use crate::handler::FemtoHandlerTrait;
use crate::{
    formatter::{DefaultFormatter, FemtoFormatter},
    log_record::FemtoLogRecord,
};

pub use config::{HandlerConfig, OverflowPolicy, TestConfig, DEFAULT_CHANNEL_CAPACITY};
use worker::{spawn_worker, FileCommand, WorkerConfig};

/// Internal items needed by the worker implementation.
mod mod_impl {
    use super::*;

    pub(super) fn write_record<W, F>(
        writer: &mut W,
        formatter: &F,
        record: FemtoLogRecord,
        flush_tracker: &mut worker::FlushTracker,
    ) -> io::Result<()>
    where
        W: Write,
        F: FemtoFormatter,
    {
        let msg = formatter.format(&record);
        writeln!(writer, "{msg}")?;
        flush_tracker.record_write(writer)
    }
}

#[pyclass]
pub struct FemtoFileHandler {
    tx: Option<Sender<FileCommand>>,
    handle: Option<JoinHandle<()>>,
    done_rx: Receiver<()>,
    overflow_policy: OverflowPolicy,
    ack_rx: Receiver<()>,
}

fn parse_overflow_policy(policy: &str, timeout_ms: Option<i64>) -> PyResult<OverflowPolicy> {
    use pyo3::exceptions::PyValueError;
    let policy = policy.trim().to_ascii_lowercase();
    match policy.as_str() {
        "drop" => {
            if timeout_ms.is_some() {
                Err(PyValueError::new_err(
                    "timeout_ms only valid for timeout policy",
                ))
            } else {
                Ok(OverflowPolicy::Drop)
            }
        }
        "block" => {
            if timeout_ms.is_some() {
                Err(PyValueError::new_err(
                    "timeout_ms only valid for timeout policy",
                ))
            } else {
                Ok(OverflowPolicy::Block)
            }
        }
        "timeout" => {
            let ms = timeout_ms
                .ok_or_else(|| PyValueError::new_err("timeout_ms required for timeout policy"))?;
            if ms <= 0 {
                return Err(PyValueError::new_err(
                    "timeout_ms must be greater than zero",
                ));
            }
            Ok(OverflowPolicy::Timeout(Duration::from_millis(ms as u64)))
        }
        other => {
            let valid = ["drop", "block", "timeout"].join(", ");
            Err(PyValueError::new_err(format!(
                "invalid overflow policy '{other}'. Valid options are: {valid}"
            )))
        }
    }
}

fn validate_params(capacity: usize, flush_interval: isize) -> PyResult<usize> {
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

fn open_log_file(path: &str) -> PyResult<File> {
    use pyo3::exceptions::PyIOError;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| PyIOError::new_err(e.to_string()))
}

#[pymethods]
impl FemtoFileHandler {
    #[new]
    #[pyo3(signature=(
        path,
        capacity = DEFAULT_CHANNEL_CAPACITY,
        flush_interval = 1,
        timeout_ms = None,
        policy = "drop"
    ))]
    fn py_new(
        path: String,
        capacity: usize,
        flush_interval: isize,
        timeout_ms: Option<i64>,
        policy: &str,
    ) -> PyResult<Self> {
        let overflow_policy = parse_overflow_policy(policy, timeout_ms)?;
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
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self::from_file(file, formatter, config))
    }

    fn from_file<F>(file: File, formatter: F, config: HandlerConfig) -> Self
    where
        F: FemtoFormatter + Send + 'static,
    {
        let worker_cfg = WorkerConfig::from(&config);
        // Use a buffered writer so flush policies control when records are
        // persisted to disk. Without buffering each write reaches the OS
        // immediately, causing premature flushes and defeating the configured
        // `flush_interval`.
        let writer = BufWriter::new(file);
        Self::build_from_worker(writer, formatter, worker_cfg, config.overflow_policy)
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

    fn build_from_worker<W, F>(
        writer: W,
        formatter: F,
        worker_cfg: WorkerConfig,
        policy: OverflowPolicy,
    ) -> Self
    where
        W: Write + Send + 'static,
        F: FemtoFormatter + Send + 'static,
    {
        let (ack_tx, ack_rx) = crossbeam_channel::unbounded();
        let (tx, done_rx, handle) = spawn_worker(writer, formatter, worker_cfg, ack_tx);
        Self {
            tx: Some(tx),
            handle: Some(handle),
            done_rx,
            overflow_policy: policy,
            ack_rx,
        }
    }

    pub fn with_writer_for_test<W, F>(config: TestConfig<W, F>) -> Self
    where
        W: Write + Send + 'static,
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
        let mut worker_cfg = WorkerConfig::from(&HandlerConfig {
            capacity,
            flush_interval,
            overflow_policy,
        });
        worker_cfg.start_barrier = start_barrier;
        Self::build_from_worker(writer, formatter, worker_cfg, overflow_policy)
    }
}

impl FemtoHandlerTrait for FemtoFileHandler {
    fn handle(&self, record: FemtoLogRecord) {
        if let Some(tx) = &self.tx {
            match self.overflow_policy {
                OverflowPolicy::Drop => {
                    if tx.try_send(FileCommand::Record(record)).is_err() {
                        log::warn!(
                            "FemtoFileHandler (Drop): queue full or shutting down, dropping record"
                        );
                    }
                }
                OverflowPolicy::Block => {
                    if tx.send(FileCommand::Record(record)).is_err() {
                        log::warn!(
                            "FemtoFileHandler (Block): queue full or shutting down, dropping record"
                        );
                    }
                }
                OverflowPolicy::Timeout(dur) => {
                    if tx.send_timeout(FileCommand::Record(record), dur).is_err() {
                        log::warn!(
                            "FemtoFileHandler (Timeout): timed out waiting for queue, dropping record"
                        );
                    }
                }
            }
        } else {
            log::warn!("FemtoFileHandler: handle called after close");
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
