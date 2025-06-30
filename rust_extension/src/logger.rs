#![allow(non_local_definitions)]

use pyo3::prelude::*;

use crossbeam_channel::{bounded, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::{
    formatter::{DefaultFormatter, FemtoFormatter},
    log_record::FemtoLogRecord,
};
use std::sync::Arc;

const DEFAULT_CHANNEL_CAPACITY: usize = 1024;

enum LoggerCommand {
    Record(FemtoLogRecord),
    Shutdown,
}

#[pyclass]
pub struct FemtoLogger {
    /// Identifier used to distinguish log messages from different loggers.
    name: String,
    formatter: Arc<dyn FemtoFormatter>,
    tx: Option<Sender<LoggerCommand>>,
    handle: Option<JoinHandle<()>>,
    done_rx: Receiver<()>,
}

#[pymethods]
impl FemtoLogger {
    /// Create a new logger with the given name.
    #[new]
    #[pyo3(text_signature = "(name)")]
    pub fn new(name: String) -> Self {
        // Use a bounded channel to prevent unbounded memory growth if log
        // producers outpace the consumer thread.
        let (tx, rx): (Sender<LoggerCommand>, Receiver<LoggerCommand>) =
            bounded(DEFAULT_CHANNEL_CAPACITY);
        let (done_tx, done_rx) = bounded(1);

        // Default to a simple formatter using the "name [LEVEL] message" style.
        let formatter: Arc<dyn FemtoFormatter> = Arc::new(DefaultFormatter);
        let thread_formatter = Arc::clone(&formatter);

        let handle = thread::spawn(move || {
            for cmd in rx {
                match cmd {
                    LoggerCommand::Record(record) => {
                        println!("{}", thread_formatter.format(&record));
                    }
                    LoggerCommand::Shutdown => break,
                }
            }
            let _ = done_tx.send(());
        });

        Self {
            name,
            formatter,
            tx: Some(tx),
            handle: Some(handle),
            done_rx,
        }
    }

    /// Format a message at the provided level and return it.
    ///
    /// This method currently builds a simple string combining the logger's
    /// name with the level and message.
    #[pyo3(text_signature = "(self, level, message)")]
    pub fn log(&self, level: &str, message: &str) -> String {
        let record = FemtoLogRecord::new(&self.name, level, message);
        let msg = self.formatter.format(&record);
        if let Some(tx) = &self.tx {
            if tx.send(LoggerCommand::Record(record)).is_err() {
                eprintln!("Warning: failed to send log record to background thread");
            }
        }
        msg
    }
}

impl Drop for FemtoLogger {
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(LoggerCommand::Shutdown);
        }
        if let Some(handle) = self.handle.take() {
            if self.done_rx.recv_timeout(Duration::from_secs(1)).is_err() {
                eprintln!("FemtoLogger: worker thread did not shut down within 1s");
                return;
            }
            if handle.join().is_err() {
                eprintln!("FemtoLogger: worker thread panicked");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn drop_with_extra_sender() {
        let logger = FemtoLogger::new("test".to_string());
        let tx = logger.tx.as_ref().unwrap().clone();
        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            drop(tx);
        });
        let start = Instant::now();
        drop(logger);
        assert!(start.elapsed() < Duration::from_secs(1));
        handle.join().unwrap();
    }
}
