//! File-based logging handler implemented with a producer-consumer model.
//! A background thread owns the file handle and formatter, receiving
//! `FemtoLogRecord` values over a bounded channel and writing them
//! asynchronously.

use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::Path,
    sync::{Arc, Barrier},
    thread::{self, JoinHandle},
    time::Duration,
};

use crossbeam_channel::{bounded, Receiver, Sender};
use log::warn;
use pyo3::prelude::*;

use crate::handler::FemtoHandlerTrait;
use crate::{
    formatter::{DefaultFormatter, FemtoFormatter},
    log_record::FemtoLogRecord,
};

const DEFAULT_CHANNEL_CAPACITY: usize = 1024;

/// Determines how `FemtoFileHandler` reacts when its queue is full.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverflowPolicy {
    /// Drop new records, preserving existing ones. Current default behaviour.
    Drop,
    /// Block the caller until space becomes available.
    Block,
    /// Block up to the specified duration before giving up.
    Timeout(Duration),
}

/// Handler that writes formatted log records to a file on a background thread.
enum FileCommand {
    Record(FemtoLogRecord),
    Flush(Sender<()>),
}

#[pyclass]
pub struct FemtoFileHandler {
    tx: Option<Sender<FileCommand>>,
    handle: Option<JoinHandle<()>>,
    done_rx: Receiver<()>,
    overflow_policy: OverflowPolicy,
}

#[pymethods]
impl FemtoFileHandler {
    /// Python constructor mirroring `new` but raising `OSError` on failure.
    #[new]
    fn py_new(path: String) -> PyResult<Self> {
        Self::new(path).map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))
    }

    /// Construct a handler with a caller-specified queue size.
    #[staticmethod]
    #[pyo3(name = "with_capacity")]
    fn py_with_capacity(path: String, capacity: usize) -> PyResult<Self> {
        Self::with_capacity(path, DefaultFormatter, capacity)
            .map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))
    }

    /// Create a blocking handler that waits when the queue is full.
    #[staticmethod]
    #[pyo3(name = "with_capacity_blocking")]
    fn py_with_capacity_blocking(path: String, capacity: usize) -> PyResult<Self> {
        Self::with_capacity_flush_policy(path, DefaultFormatter, capacity, 1, OverflowPolicy::Block)
            .map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))
    }

    /// Create a timeout-based handler. `timeout_ms` specifies how long to wait for space.
    #[staticmethod]
    #[pyo3(name = "with_capacity_timeout")]
    fn py_with_capacity_timeout(path: String, capacity: usize, timeout_ms: u64) -> PyResult<Self> {
        Self::with_capacity_flush_policy(
            path,
            DefaultFormatter,
            capacity,
            1,
            OverflowPolicy::Timeout(Duration::from_millis(timeout_ms)),
        )
        .map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))
    }

    /// Create a handler with a custom flush interval.
    ///
    /// `flush_interval` controls how often the worker thread flushes the
    /// underlying file. A value of `0` disables periodic flushing and only
    /// flushes when the handler shuts down.
    #[staticmethod]
    #[pyo3(name = "with_capacity_flush")]
    fn py_with_capacity_flush(
        path: String,
        capacity: usize,
        flush_interval: usize,
    ) -> PyResult<Self> {
        Self::with_capacity_flush_interval(path, DefaultFormatter, capacity, flush_interval)
            .map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))
    }

    /// Blocking variant of `with_capacity_flush`.
    #[staticmethod]
    #[pyo3(name = "with_capacity_flush_blocking")]
    fn py_with_capacity_flush_blocking(
        path: String,
        capacity: usize,
        flush_interval: usize,
    ) -> PyResult<Self> {
        Self::with_capacity_flush_policy(
            path,
            DefaultFormatter,
            capacity,
            flush_interval,
            OverflowPolicy::Block,
        )
        .map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))
    }

    /// Timeout variant of `with_capacity_flush`.
    #[staticmethod]
    #[pyo3(name = "with_capacity_flush_timeout")]
    fn py_with_capacity_flush_timeout(
        path: String,
        capacity: usize,
        flush_interval: usize,
        timeout_ms: u64,
    ) -> PyResult<Self> {
        Self::with_capacity_flush_policy(
            path,
            DefaultFormatter,
            capacity,
            flush_interval,
            OverflowPolicy::Timeout(Duration::from_millis(timeout_ms)),
        )
        .map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))
    }

    /// Dispatch a log record created from the provided parameters.
    #[pyo3(name = "handle")]
    fn py_handle(&self, logger: &str, level: &str, message: &str) {
        <Self as FemtoHandlerTrait>::handle(self, FemtoLogRecord::new(logger, level, message));
    }

    /// Flush pending log records without shutting down the worker thread.
    #[pyo3(name = "flush")]
    fn py_flush(&self) -> bool {
        self.flush()
    }

    /// Close the handler and wait for the worker thread to finish.
    #[pyo3(name = "close")]
    fn py_close(&mut self) {
        self.close();
    }
}
impl FemtoFileHandler {
    /// Convenience constructor using the default formatter and queue capacity.
    /// Spawn the worker thread that processes file commands.
    fn spawn_worker<W, F>(
        writer: W,
        formatter: F,
        capacity: usize,
        flush_interval: usize,
        _overflow_policy: OverflowPolicy,
        start_barrier: Option<Arc<Barrier>>,
    ) -> (Sender<FileCommand>, Receiver<()>, JoinHandle<()>)
    where
        W: Write + Send + 'static,
        F: FemtoFormatter + Send + 'static,
    {
        let (tx, rx) = bounded(capacity);
        let (done_tx, done_rx) = bounded(1);
        let handle = thread::spawn(move || {
            if let Some(b) = start_barrier {
                b.wait();
            }
            let mut writer = writer;
            let formatter = formatter;
            let mut writes = 0usize;
            for cmd in rx {
                match cmd {
                    FileCommand::Record(record) => {
                        let msg = formatter.format(&record);
                        if writeln!(writer, "{msg}")
                            .and_then(|_| writer.flush())
                            .is_err()
                        {
                            warn!("FemtoFileHandler write error");
                        } else {
                            writes += 1;
                            if flush_interval != 0
                                && writes % flush_interval == 0
                                && writer.flush().is_err()
                            {
                                warn!("FemtoFileHandler flush error");
                            }
                        }
                    }
                    FileCommand::Flush(ack) => {
                        if writer.flush().is_err() {
                            warn!("FemtoFileHandler flush error");
                        }
                        writes = 0;
                        let _ = ack.send(());
                    }
                }
            }
            if writer.flush().is_err() {
                warn!("FemtoFileHandler flush error");
            }
            let _ = done_tx.send(());
        });
        (tx, done_rx, handle)
    }
    pub fn new<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        Self::with_capacity(path, DefaultFormatter, DEFAULT_CHANNEL_CAPACITY)
    }

    /// Create a new handler with a custom formatter and bounded queue size.
    ///
    /// `capacity` controls the length of the internal channel used to pass
    /// records to the worker thread. When full, new records are dropped.
    pub fn with_capacity<P, F>(path: P, formatter: F, capacity: usize) -> io::Result<Self>
    where
        P: AsRef<Path>,
        F: FemtoFormatter + Send + 'static,
    {
        Self::with_capacity_flush_policy(path, formatter, capacity, 1, OverflowPolicy::Drop)
    }

    /// Create a new handler with custom capacity and flush interval.
    ///
    /// `flush_interval` determines how many records are written before the
    /// worker thread flushes the file. A value of `0` disables periodic flushes
    /// and only flushes on shutdown.
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
        Self::with_capacity_flush_policy(
            path,
            formatter,
            capacity,
            flush_interval,
            OverflowPolicy::Drop,
        )
    }

    /// Create a handler with explicit overflow policy.
    pub fn with_capacity_flush_policy<P, F>(
        path: P,
        formatter: F,
        capacity: usize,
        flush_interval: usize,
        overflow_policy: OverflowPolicy,
    ) -> io::Result<Self>
    where
        P: AsRef<Path>,
        F: FemtoFormatter + Send + 'static,
    {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self::from_file(
            file,
            formatter,
            capacity,
            flush_interval,
            overflow_policy,
        ))
    }

    /// Build a handler using an already opened `File` and custom formatter.
    ///
    /// This is primarily used by `with_capacity` after opening the file.
    fn from_file<F>(
        file: File,
        formatter: F,
        capacity: usize,
        flush_interval: usize,
        overflow_policy: OverflowPolicy,
    ) -> Self
    where
        F: FemtoFormatter + Send + 'static,
    {
        let (tx, done_rx, handle) = Self::spawn_worker(
            file,
            formatter,
            capacity,
            flush_interval,
            overflow_policy,
            None,
        );
        Self {
            tx: Some(tx),
            handle: Some(handle),
            done_rx,
            overflow_policy,
        }
    }

    /// Flush any pending log records.
    pub fn flush(&self) -> bool {
        if let Some(tx) = &self.tx {
            let (ack_tx, ack_rx) = bounded(1);
            if tx.send(FileCommand::Flush(ack_tx)).is_err() {
                return false;
            }
            return ack_rx.recv_timeout(Duration::from_secs(1)).is_ok();
        }
        false
    }

    /// Close the handler and wait for the worker thread to exit.
    pub fn close(&mut self) {
        self.tx.take();
        if let Some(handle) = self.handle.take() {
            if self.done_rx.recv_timeout(Duration::from_secs(1)).is_err() {
                warn!("FemtoFileHandler: worker thread did not shut down within 1s");
                return;
            }
            if handle.join().is_err() {
                warn!("FemtoFileHandler: worker thread panicked");
            }
        }
    }
}

impl FemtoHandlerTrait for FemtoFileHandler {
    /// Send a `FemtoLogRecord` to the worker thread.
    ///
    /// The call never blocks. If the queue is full, the record is dropped and a
    /// warning is emitted via the `log` crate.
    fn handle(&self, record: FemtoLogRecord) {
        if let Some(tx) = &self.tx {
            let (failed, reason) = match self.overflow_policy {
                OverflowPolicy::Drop => (
                    tx.try_send(FileCommand::Record(record)).is_err(),
                    "queue full or shutting down",
                ),
                OverflowPolicy::Block => (
                    tx.send(FileCommand::Record(record)).is_err(),
                    "queue full or shutting down",
                ),
                OverflowPolicy::Timeout(dur) => (
                    tx.send_timeout(FileCommand::Record(record), dur).is_err(),
                    "timeout while sending to queue",
                ),
            };
            if failed {
                warn!("FemtoFileHandler: {reason}, dropping record");
            }
        } else {
            warn!("FemtoFileHandler: handle called after close");
        }
    }
}

impl Drop for FemtoFileHandler {
    /// Wait for the worker thread to finish processing remaining records.
    ///
    /// If the thread does not confirm shutdown within one second, a warning is
    /// logged and the handler drops without joining the thread.
    fn drop(&mut self) {
        self.close();
    }
}

impl FemtoFileHandler {
    /// Construct a handler from an arbitrary writer for testing.
    pub fn with_writer_for_test<W, F>(
        writer: W,
        formatter: F,
        capacity: usize,
        flush_interval: usize,
        overflow_policy: OverflowPolicy,
        start_barrier: Option<std::sync::Arc<std::sync::Barrier>>,
    ) -> Self
    where
        W: Write + Send + 'static,
        F: FemtoFormatter + Send + 'static,
    {
        let (tx, done_rx, handle) = Self::spawn_worker(
            writer,
            formatter,
            capacity,
            flush_interval,
            overflow_policy,
            start_barrier,
        );
        Self {
            tx: Some(tx),
            handle: Some(handle),
            done_rx,
            overflow_policy,
        }
    }
}
