#![allow(non_local_definitions)]

use pyo3::prelude::*;

use crossbeam_channel::{unbounded, Receiver, Sender};
use std::thread::{self, JoinHandle};

use crate::log_record::FemtoLogRecord;

/// Basic logger used for early experimentation.
#[pyclass]
pub struct FemtoLogger {
    /// Identifier used to distinguish log messages from different loggers.
    name: String,
    tx: Option<Sender<FemtoLogRecord>>,
    handle: Option<JoinHandle<()>>,
}

#[pymethods]
impl FemtoLogger {
    /// Create a new logger with the given name.
    #[new]
    #[pyo3(text_signature = "(name)")]
    pub fn new(name: String) -> Self {
        let (tx, rx): (Sender<FemtoLogRecord>, Receiver<FemtoLogRecord>) = unbounded();
        let handle = thread::spawn(move || {
            for record in rx {
                println!("{}", record);
            }
        });
        Self {
            name,
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
        let record = FemtoLogRecord::new(level, message);
        let msg = format!("{}: {}", self.name, record);
        if let Some(tx) = &self.tx {
            if tx.send(record).is_err() {
                eprintln!("Warning: failed to send log record to background thread");
            }
        }
        msg
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
