use std::{
    io::{self, Write},
    sync::mpsc,
    thread::{self, JoinHandle},
    time::Duration,
};

use crossbeam_channel::{bounded, Sender};
use pyo3::prelude::*;

use crate::handler::FemtoHandler;
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
#[pyclass]
pub struct FemtoStreamHandler {
    tx: Sender<FemtoLogRecord>,
    handle: Option<JoinHandle<()>>,
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
        let handle = thread::spawn(move || {
            let mut writer = writer;
            let formatter = formatter;
            for record in rx {
                let msg = formatter.format(&record);
                if writeln!(writer, "{}", msg)
                    .and_then(|_| writer.flush())
                    .is_err()
                {
                    eprintln!("FemtoStreamHandler write error");
                }
            }
        });

        Self {
            tx,
            handle: Some(handle),
        }
    }
}

impl FemtoHandler for FemtoStreamHandler {
    fn handle(&self, record: FemtoLogRecord) {
        if self.tx.try_send(record).is_err() {
            eprintln!("FemtoStreamHandler: queue full or shutting down, dropping record");
        }
    }
}

impl Drop for FemtoStreamHandler {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            // Joining may block if the worker misbehaves. Spawn a helper
            // thread so drop returns even if the worker is stuck.
            let (tx, rx) = mpsc::channel();
            thread::spawn(move || {
                let _ = handle.join();
                let _ = tx.send(());
            });
            let _ = rx.recv_timeout(Duration::from_secs(1));
        }
    }
}
