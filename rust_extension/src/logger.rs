//! Core logger implementation for the FemtoLogger system.
//!
//! This module provides the [`FemtoLogger`] struct which handles log message
//! filtering, formatting, and asynchronous output via a background thread.

// FIXME: Track PyO3 issue for proper fix
use pyo3::prelude::*;
use pyo3::{Py, PyAny};

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

/// Wrapper allowing Python handler objects to be used by the logger.
struct PyHandler {
    obj: Py<PyAny>,
}

impl FemtoHandlerTrait for PyHandler {
    fn handle(&self, record: FemtoLogRecord) {
        Python::with_gil(|py| {
            if let Err(err) = self.obj.call_method1(
                py,
                "handle",
                (&record.logger, &record.level, &record.message),
            ) {
                err.print(py);
                warn!("PyHandler: error calling handle");
            }
        });
    }
}

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

    /// Attach a handler implemented in Python or Rust.
    #[pyo3(name = "add_handler", text_signature = "(self, handler)")]
    pub fn py_add_handler(&self, handler: Py<PyAny>) {
        self.add_handler(Arc::new(PyHandler { obj: handler }) as Arc<dyn FemtoHandlerTrait>);
    }
}

impl FemtoLogger {
    /// Attach a handler to this logger.
    pub fn add_handler(&self, handler: Arc<dyn FemtoHandlerTrait>) {
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
    ///
    /// Consumes all messages still available on `rx` and dispatches them
    /// through the provided `handlers`. This ensures no log records are lost
    /// during shutdown.
    ///
    /// # Arguments
    ///
    /// * `rx` - Channel receiver holding pending log records.
    /// * `handlers` - Shared collection of handlers used to process records.
    fn drain_remaining_records(
        rx: &Receiver<FemtoLogRecord>,
        handlers: &Arc<RwLock<Vec<Arc<dyn FemtoHandlerTrait>>>>,
    ) {
        while let Ok(record) = rx.try_recv() {
            Self::handle_log_record(handlers, record);
        }
    }

    /// Main loop executed by the logger's worker thread.
    ///
    /// Waits on either incoming log records or a shutdown signal using
    /// `select!`. Each received record is forwarded to `handle_log_record`.
    /// When a shutdown signal arrives, any queued records are drained before
    /// the thread exits.
    ///
    /// # Arguments
    ///
    /// * `rx` - Channel receiver for new log records.
    /// * `shutdown_rx` - Channel receiver signaling shutdown.
    /// * `handlers` - Shared collection of handlers for processing records.
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
            Python::with_gil(|py| {
                py.allow_threads(move || {
                    if handle.join().is_err() {
                        warn!("FemtoLogger: worker thread panicked");
                    }
                })
            });
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct CollectingHandler {
        records: Arc<Mutex<Vec<FemtoLogRecord>>>,
    }

    impl CollectingHandler {
        fn new() -> Self {
            Self {
                records: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn collected(&self) -> Vec<FemtoLogRecord> {
            self.records.lock().expect("Failed to lock records").clone()
        }
    }

    impl FemtoHandlerTrait for CollectingHandler {
        fn handle(&self, record: FemtoLogRecord) {
            self.records
                .lock()
                .expect("Failed to lock records")
                .push(record);
        }
    }

    #[test]
    fn handle_log_record_dispatches() {
        let h1 = Arc::new(CollectingHandler::new());
        let h2 = Arc::new(CollectingHandler::new());
        let handlers = Arc::new(RwLock::new(vec![
            h1.clone() as Arc<dyn FemtoHandlerTrait>,
            h2.clone(),
        ]));
        let record = FemtoLogRecord::new("core", "INFO", "msg");

        FemtoLogger::handle_log_record(&handlers, record);

        let r1 = h1.collected();
        let r2 = h2.collected();
        assert_eq!(r1.len(), 1);
        assert_eq!(r2.len(), 1);
        assert_eq!(r1[0].message, "msg");
        assert_eq!(r2[0].message, "msg");
    }

    #[test]
    fn drain_remaining_records_pulls_all() {
        let (tx, rx) = crossbeam_channel::bounded(4);
        for i in 0..3 {
            tx.send(FemtoLogRecord::new("core", "INFO", &format!("{i}")))
                .expect("Failed to send test record");
        }
        drop(tx);

        let h = Arc::new(CollectingHandler::new());
        let handlers = Arc::new(RwLock::new(vec![h.clone() as Arc<dyn FemtoHandlerTrait>]));

        FemtoLogger::drain_remaining_records(&rx, &handlers);

        let msgs: Vec<String> = h.collected().into_iter().map(|r| r.message).collect();
        assert_eq!(msgs, vec!["0", "1", "2"]);
    }

    #[test]
    fn worker_thread_loop_processes_and_drains() {
        let (tx, rx) = crossbeam_channel::bounded(4);
        let (shutdown_tx, shutdown_rx) = crossbeam_channel::bounded(1);
        let h = Arc::new(CollectingHandler::new());
        let handlers = Arc::new(RwLock::new(vec![h.clone() as Arc<dyn FemtoHandlerTrait>]));

        let thread = std::thread::spawn(move || {
            FemtoLogger::worker_thread_loop(rx, shutdown_rx, handlers);
        });

        tx.send(FemtoLogRecord::new("core", "INFO", "one"))
            .expect("Failed to send first test record");
        tx.send(FemtoLogRecord::new("core", "INFO", "two"))
            .expect("Failed to send second test record");
        shutdown_tx
            .send(())
            .expect("Failed to send shutdown signal");
        thread.join().expect("Worker thread panicked");

        let msgs: Vec<String> = h.collected().into_iter().map(|r| r.message).collect();
        assert_eq!(msgs, vec!["one", "two"]);
    }
}
