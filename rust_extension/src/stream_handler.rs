//! Stream-based logging handler implementation.
//!
//! This module defines `FemtoStreamHandler`, which formats log records and
//! writes them to a stream on a background thread. The handler forwards
//! `FemtoLogRecord` values over a bounded channel so the producer never blocks
//! on I/O.

use std::{
    io::{self, Write},
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

/// Handler that writes formatted log records to an `io::Write` stream.
///
/// Each instance owns a background thread which receives records via a
/// channel and writes them to the provided stream. The writer and formatter
/// are moved into that thread so the caller never locks or blocks.
enum StreamCommand {
    Record(FemtoLogRecord),
    Flush,
}

#[pyclass]
pub struct FemtoStreamHandler {
    tx: Option<Sender<StreamCommand>>,
    handle: Option<JoinHandle<()>>,
    done_rx: Receiver<()>,
    ack_rx: Receiver<()>,
}

#[pymethods]
impl FemtoStreamHandler {
    #[new]
    fn py_new() -> Self {
        Self::stderr()
    }

    #[staticmethod]
    #[pyo3(name = "stdout")]
    fn py_stdout() -> Self {
        Self::stdout()
    }

    #[staticmethod]
    #[pyo3(name = "stderr")]
    fn py_stderr() -> Self {
        Self::stderr()
    }

    /// Dispatch a log record to the handler's worker thread.
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

impl FemtoStreamHandler {
    /// Create a new handler writing to `stdout` with a `DefaultFormatter`.
    pub fn stdout() -> Self {
        Self::new(io::stdout(), DefaultFormatter)
    }

    /// Create a new handler writing to `stderr` with a `DefaultFormatter`.
    pub fn stderr() -> Self {
        Self::new(io::stderr(), DefaultFormatter)
    }

    /// Create a new handler from an arbitrary writer and formatter using the default capacity.
    pub fn new<W, F>(writer: W, formatter: F) -> Self
    where
        W: Write + Send + 'static,
        F: FemtoFormatter + Send + 'static,
    {
        Self::with_capacity(writer, formatter, DEFAULT_CHANNEL_CAPACITY)
    }

    /// Create a new handler with a custom channel capacity.
    pub fn with_capacity<W, F>(writer: W, formatter: F, capacity: usize) -> Self
    where
        W: Write + Send + 'static,
        F: FemtoFormatter + Send + 'static,
    {
        let (tx, rx) = bounded(capacity);
        let (done_tx, done_rx) = bounded(1);
        let (ack_tx, ack_rx) = bounded(1);
        let handle = thread::spawn(move || {
            let mut writer = writer;
            let formatter = formatter;
            for cmd in rx {
                match cmd {
                    StreamCommand::Record(record) => {
                        let msg = formatter.format(&record);
                        if writeln!(writer, "{msg}")
                            .and_then(|_| writer.flush())
                            .is_err()
                        {
                            warn!("FemtoStreamHandler write error");
                        }
                    }
                    StreamCommand::Flush => {
                        if writer.flush().is_err() {
                            warn!("FemtoStreamHandler flush error");
                        }
                        let _ = ack_tx.send(());
                    }
                }
            }
            if writer.flush().is_err() {
                warn!("FemtoStreamHandler flush error");
            }
            let _ = done_tx.send(());
        });

        Self {
            tx: Some(tx),
            handle: Some(handle),
            done_rx,
            ack_rx,
        }
    }

    /// Flush any pending log records.
    pub fn flush(&self) -> bool {
        <Self as FemtoHandlerTrait>::flush(self)
    }

    /// Close the handler and wait for the worker thread to exit.
    pub fn close(&mut self) {
        self.tx.take();
        if let Some(handle) = self.handle.take() {
            if self.done_rx.recv_timeout(Duration::from_secs(1)).is_err() {
                warn!("FemtoStreamHandler: worker thread did not shut down within 1s");
                return;
            }
            if handle.join().is_err() {
                warn!("FemtoStreamHandler: worker thread panicked");
            }
        }
    }
}

impl FemtoHandlerTrait for FemtoStreamHandler {
    fn handle(&self, record: FemtoLogRecord) {
        if let Some(tx) = &self.tx {
            if tx.try_send(StreamCommand::Record(record)).is_err() {
                warn!("FemtoStreamHandler: queue full or shutting down, dropping record");
            }
        } else {
            warn!("FemtoStreamHandler: handle called after close");
        }
    }

    fn flush(&self) -> bool {
        match &self.tx {
            Some(tx) => {
                if tx.send(StreamCommand::Flush).is_err() {
                    return false;
                }
                self.ack_rx.recv_timeout(Duration::from_secs(1)).is_ok()
            }
            None => false,
        }
    }
}

impl Drop for FemtoStreamHandler {
    fn drop(&mut self) {
        self.close();
    }
}
