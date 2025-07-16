//! Public API for the file-based logging handler.
//!
//! `FemtoFileHandler` spawns a dedicated worker thread that writes formatted
//! log records to disk. Configuration types and the worker implementation live
//! in submodules and are re-exported here for external use.

mod config;
mod worker;

use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
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

pub use config::{
    HandlerConfig, OverflowPolicy, PyHandlerConfig, TestConfig, DEFAULT_CHANNEL_CAPACITY,
};
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

#[pymethods]
impl FemtoFileHandler {
    #[new]
    fn py_new(path: String) -> PyResult<Self> {
        Self::new(path).map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))
    }

    #[staticmethod]
    #[pyo3(name = "with_capacity")]
    fn py_with_capacity(path: String, capacity: usize) -> PyResult<Self> {
        Self::build_py_handler(path, capacity, None, OverflowPolicy::Drop)
    }

    #[staticmethod]
    #[pyo3(name = "with_capacity_blocking")]
    fn py_with_capacity_blocking(path: String, capacity: usize) -> PyResult<Self> {
        Self::build_py_handler(path, capacity, None, OverflowPolicy::Block)
    }

    #[staticmethod]
    #[pyo3(name = "with_capacity_timeout")]
    fn py_with_capacity_timeout(path: String, capacity: usize, timeout_ms: u64) -> PyResult<Self> {
        Self::build_py_handler(
            path,
            capacity,
            None,
            OverflowPolicy::Timeout(Duration::from_millis(timeout_ms)),
        )
    }

    #[staticmethod]
    #[pyo3(name = "with_capacity_flush")]
    fn py_with_capacity_flush(
        path: String,
        capacity: usize,
        flush_interval: usize,
    ) -> PyResult<Self> {
        Self::build_py_handler(path, capacity, Some(flush_interval), OverflowPolicy::Drop)
    }

    #[staticmethod]
    #[pyo3(name = "with_capacity_flush_blocking")]
    fn py_with_capacity_flush_blocking(
        path: String,
        capacity: usize,
        flush_interval: usize,
    ) -> PyResult<Self> {
        Self::build_py_handler(path, capacity, Some(flush_interval), OverflowPolicy::Block)
    }

    #[staticmethod]
    #[pyo3(name = "with_capacity_flush_timeout")]
    fn py_with_capacity_flush_timeout(
        path: String,
        capacity: usize,
        flush_interval: usize,
        timeout_ms: u64,
    ) -> PyResult<Self> {
        Self::build_py_handler(
            path,
            capacity,
            Some(flush_interval),
            OverflowPolicy::Timeout(Duration::from_millis(timeout_ms)),
        )
    }

    #[staticmethod]
    #[pyo3(name = "with_capacity_flush_policy")]
    fn py_with_capacity_flush_policy(path: String, config: PyHandlerConfig) -> PyResult<Self> {
        use pyo3::exceptions::PyValueError;
        let policy = match config.policy.to_ascii_lowercase().as_str() {
            "drop" => OverflowPolicy::Drop,
            "block" => OverflowPolicy::Block,
            "timeout" => {
                let ms = config.timeout_ms.ok_or_else(|| {
                    PyValueError::new_err("timeout_ms required for timeout policy")
                })?;
                OverflowPolicy::Timeout(Duration::from_millis(ms))
            }
            _ => {
                let valid = ["drop", "block", "timeout"].join(", ");
                let msg = format!(
                    "invalid overflow policy: '{}'. Valid options are: {}",
                    config.policy, valid
                );
                return Err(PyValueError::new_err(msg));
            }
        };
        Self::build_py_handler(path, config.capacity, Some(config.flush_interval), policy)
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
    fn build_py_handler(
        path: String,
        capacity: usize,
        flush_interval: Option<usize>,
        overflow_policy: OverflowPolicy,
    ) -> PyResult<Self> {
        Self::handle_io_result(Self::create_with_policy(
            path,
            capacity,
            flush_interval,
            overflow_policy,
        ))
    }

    fn handle_io_result(result: io::Result<Self>) -> PyResult<Self> {
        result.map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))
    }

    fn create_with_policy<P: AsRef<Path>>(
        path: P,
        capacity: usize,
        flush_interval: Option<usize>,
        overflow_policy: OverflowPolicy,
    ) -> io::Result<Self> {
        let cfg = Self::build_config(capacity, flush_interval, overflow_policy);
        Self::with_capacity_flush_policy(path, DefaultFormatter, cfg)
    }

    pub fn new<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        Self::with_capacity(path, DefaultFormatter, DEFAULT_CHANNEL_CAPACITY)
    }

    pub fn with_capacity<P, F>(path: P, formatter: F, capacity: usize) -> io::Result<Self>
    where
        P: AsRef<Path>,
        F: FemtoFormatter + Send + 'static,
    {
        let cfg = Self::build_config(capacity, None, OverflowPolicy::Drop);
        Self::with_capacity_flush_policy(path, formatter, cfg)
    }

    pub fn with_capacity_flush_interval<P, F>(
        path: P,
        formatter: F,
        capacity: usize,
        flush_interval: usize,
    ) -> io::Result<Self>
    where
        P: AsRef<Path>,
        F: FemtoFormatter + Send + 'static,
    {
        let cfg = Self::build_config(capacity, Some(flush_interval), OverflowPolicy::Drop);
        Self::with_capacity_flush_policy(path, formatter, cfg)
    }

    pub fn with_capacity_flush_policy<P, F>(
        path: P,
        formatter: F,
        config: HandlerConfig,
    ) -> io::Result<Self>
    where
        P: AsRef<Path>,
        F: FemtoFormatter + Send + 'static,
    {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self::from_file(file, formatter, config))
    }

    fn from_file<F>(file: File, formatter: F, config: HandlerConfig) -> Self
    where
        F: FemtoFormatter + Send + 'static,
    {
        let worker_cfg = WorkerConfig::from(&config);
        Self::build_from_worker(file, formatter, worker_cfg, config.overflow_policy)
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

    fn build_config(
        capacity: usize,
        flush_interval: Option<usize>,
        overflow_policy: OverflowPolicy,
    ) -> HandlerConfig {
        let defaults = HandlerConfig::default();
        HandlerConfig {
            capacity,
            flush_interval: flush_interval.unwrap_or(defaults.flush_interval),
            overflow_policy,
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
