#![allow(non_local_definitions)]

use pyo3::prelude::*;

use crossbeam_channel::{bounded, Receiver, Sender};
use std::thread::{self, JoinHandle};

use crate::{
    formatter::{DefaultFormatter, FemtoFormatter},
    level::FemtoLevel,
    log_record::FemtoLogRecord,
};
use std::sync::Arc;

const DEFAULT_CHANNEL_CAPACITY: usize = 1024;

/// Basic logger used for early experimentation.
#[pyclass]
pub struct FemtoLogger {
    /// Identifier used to distinguish log messages from different loggers.
    name: String,
    formatter: Arc<dyn FemtoFormatter>,
    level: FemtoLevel,
    tx: Option<Sender<FemtoLogRecord>>,
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
        let (tx, rx): (Sender<FemtoLogRecord>, Receiver<FemtoLogRecord>) =
            bounded(DEFAULT_CHANNEL_CAPACITY);

        // Default to a simple formatter using the "name [LEVEL] message" style.
        let formatter: Arc<dyn FemtoFormatter> = Arc::new(DefaultFormatter);
        let thread_formatter = Arc::clone(&formatter);

        let handle = thread::spawn(move || {
            for record in rx {
                println!("{}", thread_formatter.format(&record));
            }
        });

        Self {
            name,
            formatter,
            level: FemtoLevel::Info,
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
        let record_level = FemtoLevel::parse_or_info(level);
        if record_level < self.level {
            return String::new();
        }
        let record = FemtoLogRecord::new(&self.name, level, message);
        let msg = self.formatter.format(&record);
        if let Some(tx) = &self.tx {
            if tx.send(record).is_err() {
                eprintln!("Warning: failed to send log record to background thread");
            }
        }
        msg
    }

    /// Update the logger's minimum level.
    #[pyo3(text_signature = "(self, level)")]
    pub fn set_level(&mut self, level: &str) {
        self.level = FemtoLevel::parse_or_info(level);
    }
}

impl Drop for FemtoLogger {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
