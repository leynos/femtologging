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
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

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

    /// Create a logger with an explicit parent name.
    pub fn with_parent(name: String, parent: Option<String>) -> Self {
        let formatter: Arc<dyn FemtoFormatter> = Arc::new(DefaultFormatter);

        Self {
            name,
            parent,
            formatter,
            level: AtomicU8::new(FemtoLevel::Info as u8),
            handlers: Vec::new(),
        }
    }
}
