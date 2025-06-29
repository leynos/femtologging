//! File-based logging handler implemented with a producer-consumer model.
//! A background thread owns the file handle and formatter, receiving
//! `FemtoLogRecord` values over a bounded channel and writing them
//! asynchronously.

use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::Path,
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
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self::from_file(file, formatter, capacity))
    }

    /// Build a handler using an already opened `File` and custom formatter.
    ///
    /// This is primarily used by `with_capacity` after opening the file.
    fn from_file<F>(file: File, formatter: F, capacity: usize) -> Self
    where
        F: FemtoFormatter + Send + 'static,
    {
        let (tx, rx) = bounded(capacity);
        let (done_tx, done_rx) = bounded(1);
        let handle = thread::spawn(move || {
            let mut file = file;
            let formatter = formatter;
            for cmd in rx {
                match cmd {
                    FileCommand::Record(record) => {
                        let msg = formatter.format(&record);
                        if writeln!(file, "{}", msg)
                            .and_then(|_| file.flush())
                            .is_err()
                        {
                            warn!("FemtoFileHandler write error");
                        }
                    }
                    FileCommand::Flush(ack) => {
                        if file.flush().is_err() {
                            warn!("FemtoFileHandler flush error");
                        }
                        let _ = ack.send(());
                    }
                }
            }
            let _ = done_tx.send(());
        });

        Self {
            tx: Some(tx),
            handle: Some(handle),
            done_rx,
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
            if tx.try_send(FileCommand::Record(record)).is_err() {
                warn!("FemtoFileHandler: queue full or shutting down, dropping record");
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
