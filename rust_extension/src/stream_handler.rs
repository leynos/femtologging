use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
};

use crossbeam_channel::{bounded, Sender};

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
pub struct FemtoStreamHandler {
    tx: Option<Sender<FemtoLogRecord>>,
    handle: Option<JoinHandle<()>>,
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

    /// Create a new handler from an arbitrary writer and formatter.
    pub fn new<W>(writer: Arc<Mutex<W>>, formatter: Arc<dyn FemtoFormatter>) -> Self
    where
        W: Write + Send + 'static,
    {
        let (tx, rx) = bounded(DEFAULT_CHANNEL_CAPACITY);
        let thread_writer = Arc::clone(&writer);
        let thread_formatter = formatter;

        let handle = thread::spawn(move || {
            for record in rx {
                let msg = thread_formatter.format(&record);
                if let Ok(mut w) = thread_writer.lock() {
                    let _ = writeln!(w, "{}", msg);
                    let _ = w.flush();
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
            let _ = tx.send(record);
        }
    }
}

impl Drop for FemtoStreamHandler {
    fn drop(&mut self) {
        // Dropping the sender signals the consumer thread to finish.
        self.tx.take();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
