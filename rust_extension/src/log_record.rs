//! Log record representation for the femtologging framework.
//!
//! This module defines the `FemtoLogRecord` struct that captures log events
//! along with their contextual metadata such as timestamps, source location,
//! and thread information.

use std::collections::BTreeMap;
use std::fmt;
use std::thread::{self, ThreadId};
use std::time::SystemTime;

use crate::level::FemtoLevel;

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
    /// The severity level of the record.
    pub level: FemtoLevel,
    /// The log message content.
    pub message: String,
    /// Contextual metadata for the record.
    pub metadata: RecordMetadata,
}

impl FemtoLogRecord {
    /// Construct a new log record from logger `name`, `level`, and `message`.
    pub fn new(logger: &str, level: FemtoLevel, message: &str) -> Self {
        Self {
            logger: logger.to_owned(),
            level,
            message: message.to_owned(),
            metadata: RecordMetadata::default(),
        }
    }

    /// Construct a log record with explicit source location and key-values.
    pub fn with_metadata(
        logger: &str,
        level: FemtoLevel,
        message: &str,
        mut metadata: RecordMetadata,
    ) -> Self {
        let (timestamp, thread_id, thread_name) = RecordMetadata::capture_runtime();
        metadata.timestamp = timestamp;
        metadata.thread_id = thread_id;
        metadata.thread_name = thread_name;
        Self {
            logger: logger.to_owned(),
            level,
            message: message.to_owned(),
            metadata,
        }
    }
}

impl fmt::Display for FemtoLogRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} - {}", self.level, self.message)
    }
}
