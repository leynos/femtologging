//! Log record representation for the femtologging framework.
//!
//! This module defines the `FemtoLogRecord` struct that captures log events
//! along with their contextual metadata such as timestamps, source location,
//! and thread information.

use crate::level::FemtoLevel;
use std::collections::BTreeMap;
use std::fmt;
use std::thread::{self, ThreadId};
use std::time::SystemTime;

/// Additional context associated with a log record.
#[derive(Clone, Debug)]
pub struct RecordMetadata {
    /// Rust module path where the log call originated.
    pub module_path: String,
    /// Source file name for the log call.
    pub filename: String,
    /// Line number in the source file.
    pub line_number: u32,
    /// Time the record was created.
    pub timestamp: SystemTime,
    /// ID of the thread that created the record.
    pub thread_id: ThreadId,
    /// Name of the thread that created the record (if any).
    pub thread_name: Option<String>,
    /// Structured key-value pairs attached to the record.
    pub key_values: BTreeMap<String, String>,
}

impl RecordMetadata {
    /// Capture timestamp and thread info from the current execution context.
    fn capture_runtime() -> (SystemTime, ThreadId, Option<String>) {
        let current = thread::current();
        (
            SystemTime::now(),
            current.id(),
            current.name().map(ToString::to_string),
        )
    }

    /// Return the module path where the log call originated.
    pub fn module_path(&self) -> &str {
        &self.module_path
    }

    /// Return the source file name.
    pub fn filename(&self) -> &str {
        &self.filename
    }

    /// Return the line number in the source file.
    pub fn line_number(&self) -> u32 {
        self.line_number
    }

    /// Return the timestamp when the record was created.
    pub fn timestamp(&self) -> SystemTime {
        self.timestamp
    }

    /// Return the identifier of the thread that created the record.
    pub fn thread_id(&self) -> ThreadId {
        self.thread_id
    }

    /// Return the name of the thread that created the record, if any.
    pub fn thread_name(&self) -> Option<&str> {
        self.thread_name.as_deref()
    }

    /// Return structured key-value pairs attached to the record.
    pub fn key_values(&self) -> &BTreeMap<String, String> {
        &self.key_values
    }
}

impl Default for RecordMetadata {
    fn default() -> Self {
        let (timestamp, thread_id, thread_name) = Self::capture_runtime();
        Self {
            module_path: String::new(),
            filename: String::new(),
            line_number: 0,
            timestamp,
            thread_id,
            thread_name,
            key_values: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct FemtoLogRecord {
    /// Name of the logger that created this record.
    pub logger: String,
    /// The log level as a string (e.g. "INFO" or "ERROR").
    pub level: String,
    /// Cached parsed representation of the level.
    pub parsed_level: Option<FemtoLevel>,
    /// The log message content.
    pub message: String,
    /// Contextual metadata for the record.
    pub metadata: RecordMetadata,
}

impl FemtoLogRecord {
    /// Construct a new log record from logger `name`, `level`, and `message`.
    pub fn new(logger: &str, level: &str, message: &str) -> Self {
        Self {
            logger: logger.to_owned(),
            level: level.to_owned(),
            parsed_level: level.parse().ok(),
            message: message.to_owned(),
            metadata: RecordMetadata::default(),
        }
    }

    /// Construct a log record with explicit source location and key-values.
    pub fn with_metadata(
        logger: &str,
        level: &str,
        message: &str,
        mut metadata: RecordMetadata,
    ) -> Self {
        let (timestamp, thread_id, thread_name) = RecordMetadata::capture_runtime();
        metadata.timestamp = timestamp;
        metadata.thread_id = thread_id;
        metadata.thread_name = thread_name;
        Self {
            logger: logger.to_owned(),
            level: level.to_owned(),
            parsed_level: level.parse().ok(),
            message: message.to_owned(),
            metadata,
        }
    }

    /// Return the name of the originating logger.
    pub fn logger(&self) -> &str {
        &self.logger
    }

    /// Return the log level as a string.
    pub fn level(&self) -> &str {
        &self.level
    }

    /// Return the log message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Return the record metadata.
    pub fn metadata(&self) -> &RecordMetadata {
        &self.metadata
    }
}

impl fmt::Display for FemtoLogRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} - {}", self.level, self.message)
    }
}
