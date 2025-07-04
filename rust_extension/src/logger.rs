#![allow(non_local_definitions)]

use pyo3::prelude::*;

use crossbeam_channel::{bounded, Receiver, Sender};
use std::thread::{self, JoinHandle};

use crate::{
    formatter::{DefaultFormatter, FemtoFormatter},
    log_record::FemtoLogRecord,
};
use std::sync::Arc;

/// Commands sent to the logger's worker thread.
enum LoggerCommand {
    Record(FemtoLogRecord),
    Shutdown,
}

const DEFAULT_CHANNEL_CAPACITY: usize = 1024;

/// Basic logger used for early experimentation.
#[pyclass]
pub struct FemtoLogger {
    /// Identifier used to distinguish log messages from different loggers.
    name: String,
    formatter: Arc<dyn FemtoFormatter>,
    tx: Option<Sender<LoggerCommand>>,
    handle: Option<JoinHandle<()>>,
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
        });

        Self {
            name,
            formatter,
            tx: Some(tx),
            handle: Some(handle),
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
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::thread;
    use std::time::Duration;

    /// Dropping the logger should not block even if additional `Sender` clones
    /// exist. The background thread should exit promptly once a shutdown
    /// message is sent.
    #[test]
    fn drop_does_not_block_with_live_sender() {
        let logger = FemtoLogger::new("test".to_string());
        let extra = logger.tx.as_ref().unwrap().clone();

        let (done_tx, done_rx) = channel();
        let handle = thread::spawn(move || {
            drop(logger);
            let _ = done_tx.send(());
        });

        // The drop call should finish within 200 ms despite `extra` keeping the
        // channel alive because the logger sends a shutdown command.
        let result = done_rx.recv_timeout(Duration::from_millis(200));

        // clean up to avoid hanging the test
        drop(extra);
        handle.join().unwrap();

        assert!(
            result.is_ok(),
            "dropping FemtoLogger hung while senders lived"
        );
    }
    /// Sending using a leftover sender after the logger has been dropped should
    /// return an error rather than block.
    #[test]
    fn send_after_drop_fails() {
        let logger = FemtoLogger::new("test".to_string());
        let extra = logger.tx.as_ref().unwrap().clone();

        drop(logger);

        let record = FemtoLogRecord::new("test", "INFO", "after");
        assert!(extra.send(LoggerCommand::Record(record)).is_err());
    }
}
