//! Core logger implementation for the FemtoLogger system.
//!
//! This module provides the [`FemtoLogger`] struct which handles log message
//! filtering, formatting, and asynchronous output via a background thread.

// FIXME: Track PyO3 issue for proper fix
use pyo3::prelude::*;

use crate::handler::FemtoHandlerTrait;

use crate::{
    formatter::{DefaultFormatter, FemtoFormatter},
    level::FemtoLevel,
    log_record::FemtoLogRecord,
};
use crossbeam_channel::{bounded, select, Receiver, Sender};
use log::warn;
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const DEFAULT_CHANNEL_CAPACITY: usize = 1024;

/// Basic logger used for early experimentation.
#[pyclass]
pub struct FemtoLogger {
    /// Identifier used to distinguish log messages from different loggers.
    name: String,
    /// Parent logger name for dotted hierarchy.
    #[pyo3(get)]
    parent: Option<String>,
    formatter: Arc<dyn FemtoFormatter>,
    level: AtomicU8,
    handlers: Vec<Arc<dyn FemtoHandlerTrait>>,
    tx: Option<Sender<FemtoLogRecord>>,
    shutdown_tx: Option<Sender<()>>,
    done_rx: Receiver<()>,
    handle: Option<JoinHandle<()>>,
}

#[pymethods]
impl FemtoLogger {
    /// Create a new logger with the given name.
    #[new]
    #[pyo3(text_signature = "(name)")]
    pub fn new(name: String) -> Self {
        Self::with_parent(name, None)
    }

    /// Format a message at the provided level and return it.
    ///
    /// This method currently builds a simple string combining the logger's
    /// name with the level and message.
    #[pyo3(text_signature = "(self, level, message)")]
    pub fn log(&self, level: &str, message: &str) -> Option<String> {
        let record_level = FemtoLevel::parse_or_warn(level);
        let threshold = self.level.load(Ordering::Relaxed);
        if (record_level as u8) < threshold {
            return None;
        }
        let record = FemtoLogRecord::new(&self.name, level, message);
        let msg = self.formatter.format(&record);
        for h in &self.handlers {
            h.handle(record.clone());
        }
        if let Some(tx) = &self.tx {
            if tx.send(record).is_err() {
                warn!("FemtoLogger: failed to send log record to worker");
            }
        }
        Some(msg)
    }

    /// Update the logger's minimum level.
    ///
    /// `level` accepts "TRACE", "DEBUG", "INFO", "WARN", "ERROR", or
    /// "CRITICAL". The update is thread‑safe because the level is stored in an
    /// `AtomicU8`.
    #[pyo3(text_signature = "(self, level)")]
    pub fn set_level(&self, level: &str) {
        let lvl = FemtoLevel::parse_or_warn(level);
        self.level.store(lvl as u8, Ordering::Relaxed);
    }
}

impl FemtoLogger {
    /// Attach a handler to this logger.
    pub fn add_handler(&mut self, handler: Arc<dyn FemtoHandlerTrait>) {
        self.handlers.push(handler);
    }

    #[cfg(feature = "test-util")]
    pub fn clone_sender_for_test(&self) -> Option<Sender<FemtoLogRecord>> {
        self.tx.as_ref().cloned()
    }

    /// Create a logger with an explicit parent name.
    pub fn with_parent(name: String, parent: Option<String>) -> Self {
        let formatter: Arc<dyn FemtoFormatter> = Arc::new(DefaultFormatter);

        let (tx, rx) = bounded(DEFAULT_CHANNEL_CAPACITY);
        let (shutdown_tx, shutdown_rx) = bounded(1);
        let (done_tx, done_rx) = bounded(1);
        let fmt = Arc::clone(&formatter);
        let handle = thread::spawn(move || {
            loop {
                select! {
                    recv(shutdown_rx) -> _ => {
                        while let Ok(record) = rx.try_recv() {
                            let msg = fmt.format(&record);
                            println!("{msg}");
                        }
                        break;
                    }
                    recv(rx) -> rec => match rec {
                        Ok(record) => {
                            let msg = fmt.format(&record);
                            println!("{msg}");
                        }
                        Err(_) => break,
                    }
                }
            }
            let _ = done_tx.send(());
        });

        Self {
            name,
            parent,
            formatter,
            level: AtomicU8::new(FemtoLevel::Info as u8),
            handlers: Vec::new(),
            tx: Some(tx),
            shutdown_tx: Some(shutdown_tx),
            done_rx,
            handle: Some(handle),
        }
    }
}

impl Drop for FemtoLogger {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            if self.done_rx.recv_timeout(Duration::from_secs(1)).is_err() {
                warn!("FemtoLogger: worker thread did not shut down within 1s");
                return;
            }
            if handle.join().is_err() {
                warn!("FemtoLogger: worker thread panicked");
            }
        }
    }
}
