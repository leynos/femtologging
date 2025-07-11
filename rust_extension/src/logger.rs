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
use parking_lot::RwLock;
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};
use std::thread::{self, JoinHandle};

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
    handlers: Arc<RwLock<Vec<Arc<dyn FemtoHandlerTrait>>>>,
    tx: Option<Sender<FemtoLogRecord>>,
    shutdown_tx: Option<Sender<()>>,
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
    pub fn log(&self, level: FemtoLevel, message: &str) -> Option<String> {
        let threshold = self.level.load(Ordering::Relaxed);
        if (level as u8) < threshold {
            return None;
        }
        let record = FemtoLogRecord::new(&self.name, &level.to_string(), message);
        let msg = self.formatter.format(&record);
        if let Some(tx) = &self.tx {
            if tx.try_send(record).is_err() {
                warn!("FemtoLogger: queue full or shutting down, dropping record");
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
    pub fn set_level(&self, level: FemtoLevel) {
        self.level.store(level as u8, Ordering::Relaxed);
    }
}

impl FemtoLogger {
    /// Attach a handler to this logger.
    pub fn add_handler(&mut self, handler: Arc<dyn FemtoHandlerTrait>) {
        self.handlers.write().push(handler);
    }

    /// Clone the internal sender for use in tests.
    ///
    /// # Warning
    /// Any cloned sender must be dropped before the logger can shut down.
    /// Holding a clone alive after dropping the logger will prevent the worker
    /// thread from exiting.
    #[cfg(feature = "test-util")]
    pub fn clone_sender_for_test(&self) -> Option<Sender<FemtoLogRecord>> {
        self.tx.as_ref().cloned()
    }

    /// Create a logger with an explicit parent name.
    pub fn with_parent(name: String, parent: Option<String>) -> Self {
        let formatter: Arc<dyn FemtoFormatter> = Arc::new(DefaultFormatter);
        let handlers: Arc<RwLock<Vec<Arc<dyn FemtoHandlerTrait>>>> =
            Arc::new(RwLock::new(Vec::new()));

        let (tx, rx) = bounded::<FemtoLogRecord>(DEFAULT_CHANNEL_CAPACITY);
        let (shutdown_tx, shutdown_rx) = bounded::<()>(1);
        let thread_handlers = Arc::clone(&handlers);
        let handle = thread::spawn(move || {
            Self::worker_thread_loop(rx, shutdown_rx, thread_handlers);
        });

        Self {
            name,
            parent,
            formatter,
            level: AtomicU8::new(FemtoLevel::Info as u8),
            handlers,
            tx: Some(tx),
            shutdown_tx: Some(shutdown_tx),
            handle: Some(handle),
        }
    }

    /// Process a single `FemtoLogRecord` by dispatching it to all handlers.
    fn handle_log_record(
        handlers: &Arc<RwLock<Vec<Arc<dyn FemtoHandlerTrait>>>>,
        record: FemtoLogRecord,
    ) {
        for h in handlers.read().iter() {
            h.handle(record.clone());
        }
    }

    /// Drain any remaining records once a shutdown signal is received.
    fn drain_remaining_records(
        rx: &Receiver<FemtoLogRecord>,
        handlers: &Arc<RwLock<Vec<Arc<dyn FemtoHandlerTrait>>>>,
    ) {
        while let Ok(record) = rx.try_recv() {
            Self::handle_log_record(handlers, record);
        }
    }

    /// Main loop executed by the logger's worker thread.
    fn worker_thread_loop(
        rx: Receiver<FemtoLogRecord>,
        shutdown_rx: Receiver<()>,
        handlers: Arc<RwLock<Vec<Arc<dyn FemtoHandlerTrait>>>>,
    ) {
        loop {
            select! {
                recv(rx) -> rec => match rec {
                    Ok(record) => Self::handle_log_record(&handlers, record),
                    Err(_) => break,
                },
                recv(shutdown_rx) -> _ => {
                    Self::drain_remaining_records(&rx, &handlers);
                    break;
                }
            }
        }
    }
}

impl Drop for FemtoLogger {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        self.tx.take();
        if let Some(handle) = self.handle.take() {
            if handle.join().is_err() {
                warn!("FemtoLogger: worker thread panicked");
            }
        }
    }
}
