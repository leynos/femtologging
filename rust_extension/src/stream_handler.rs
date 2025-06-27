use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
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
/// channel and writes them to the provided stream. The stream is protected
/// by a `Mutex` to avoid interleaved writes when shared across threads.
#[pyclass]
pub struct FemtoStreamHandler {
    tx: Option<Sender<FemtoLogRecord>>,
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
        Self::new(
            Arc::new(Mutex::new(io::stdout())),
            Arc::new(DefaultFormatter),
        )
    }

    /// Create a new handler writing to `stderr` with a `DefaultFormatter`.
    pub fn stderr() -> Self {
        Self::new(
            Arc::new(Mutex::new(io::stderr())),
            Arc::new(DefaultFormatter),
        )
    }

    /// Create a new handler from an arbitrary writer and formatter using the default capacity.
    pub fn new<W>(writer: Arc<Mutex<W>>, formatter: Arc<dyn FemtoFormatter>) -> Self
    where
        W: Write + Send + 'static,
    {
        Self::with_capacity(writer, formatter, DEFAULT_CHANNEL_CAPACITY)
    }

    /// Create a new handler with a custom channel capacity.
    pub fn with_capacity<W>(
        writer: Arc<Mutex<W>>,
        formatter: Arc<dyn FemtoFormatter>,
        capacity: usize,
    ) -> Self
    where
        W: Write + Send + 'static,
    {
        let (tx, rx) = bounded(capacity);
        let thread_writer = Arc::clone(&writer);
        let thread_formatter = formatter;

        let handle = thread::spawn(move || {
            for record in rx {
                let msg = thread_formatter.format(&record);
                match thread_writer.lock() {
                    Ok(mut w) => {
                        if let Err(e) = writeln!(w, "{}", msg) {
                            eprintln!("FemtoStreamHandler write error: {}", e);
                        }
                        if let Err(e) = w.flush() {
                            eprintln!("FemtoStreamHandler flush error: {}", e);
                        }
                    }
                    Err(e) => {
                        eprintln!("FemtoStreamHandler lock error: {}", e);
                    }
                }
            }
        });

        Self {
            tx: Some(tx),
            handle: Some(handle),
        }
    }
}

impl FemtoHandler for FemtoStreamHandler {
    fn handle(&self, record: FemtoLogRecord) {
        if let Some(tx) = &self.tx {
            if tx.try_send(record).is_err() {
                eprintln!("FemtoStreamHandler: queue full or shutting down, dropping record");
            }
        }
    }
}

impl Drop for FemtoStreamHandler {
    fn drop(&mut self) {
        // Dropping the sender signals the consumer thread to finish.
        if let Some(sender) = self.tx.take() {
            drop(sender);
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
