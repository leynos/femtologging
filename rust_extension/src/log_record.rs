//! Log record representation for the femtologging framework.
//!
//! This module defines the `FemtoLogRecord` struct that captures log events
//! along with their contextual metadata such as timestamps, source location,
//! and thread information.

use crate::exception_schema::{ExceptionPayload, StackTracePayload};
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
    logger: String,
    /// The log level for this record.
    level: FemtoLevel,
    /// The log message content.
    message: String,
    /// Contextual metadata for the record.
    metadata: RecordMetadata,
    /// Structured exception payload (when `exc_info` is provided).
    exception_payload: Option<ExceptionPayload>,
    /// Structured stack trace payload (when `stack_info=True`).
    stack_payload: Option<StackTracePayload>,
}

impl FemtoLogRecord {
    /// Returns the logger name.
    #[inline]
    pub fn logger(&self) -> &str {
        &self.logger
    }

    /// Returns the log level.
    #[inline]
    pub fn level(&self) -> FemtoLevel {
        self.level
    }

    /// Returns the log message.
    #[inline]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns a reference to the record metadata.
    #[inline]
    pub fn metadata(&self) -> &RecordMetadata {
        &self.metadata
    }

    /// Returns a reference to the exception payload, if present.
    #[inline]
    pub fn exception_payload(&self) -> Option<&ExceptionPayload> {
        self.exception_payload.as_ref()
    }

    /// Returns a reference to the stack trace payload, if present.
    #[inline]
    pub fn stack_payload(&self) -> Option<&StackTracePayload> {
        self.stack_payload.as_ref()
    }

    /// Sets the exception payload.
    #[inline]
    #[cfg(feature = "python")]
    pub(crate) fn set_exception_payload(&mut self, payload: ExceptionPayload) {
        self.exception_payload = Some(payload);
    }

    /// Sets the stack trace payload.
    #[inline]
    #[cfg(feature = "python")]
    pub(crate) fn set_stack_payload(&mut self, payload: StackTracePayload) {
        self.stack_payload = Some(payload);
    }
}

impl FemtoLogRecord {
    /// Construct a new log record from logger `name`, `level`, and `message`.
    pub fn new(logger: &str, level: FemtoLevel, message: &str) -> Self {
        Self {
            logger: logger.to_owned(),
            level,
            message: message.to_owned(),
            metadata: RecordMetadata::default(),
            exception_payload: None,
            stack_payload: None,
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
            exception_payload: None,
            stack_payload: None,
        }
    }

    /// Return the level name as a static string slice.
    ///
    /// This is a zero-cost accessor that returns the canonical level name
    /// (e.g., "INFO", "ERROR") without allocation.
    #[inline]
    pub fn level_str(&self) -> &'static str {
        self.level.as_str()
    }

    /// Attach an exception payload to the record.
    #[must_use]
    pub fn with_exception(mut self, payload: ExceptionPayload) -> Self {
        self.exception_payload = Some(payload);
        self
    }

    /// Attach a stack trace payload to the record.
    #[must_use]
    pub fn with_stack(mut self, payload: StackTracePayload) -> Self {
        self.stack_payload = Some(payload);
        self
    }
}

impl fmt::Display for FemtoLogRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} - {}", self.level_str(), self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exception_schema::{ExceptionPayload, StackFrame, StackTracePayload};
    use rstest::rstest;

    #[rstest]
    fn new_record_has_no_payloads() {
        let record = FemtoLogRecord::new("test", FemtoLevel::Info, "message");
        assert!(record.exception_payload.is_none());
        assert!(record.stack_payload.is_none());
    }

    #[rstest]
    fn with_exception_attaches_payload() {
        let exc = ExceptionPayload::new("ValueError", "bad input");
        let record = FemtoLogRecord::new("test", FemtoLevel::Error, "failed").with_exception(exc);

        assert!(record.exception_payload.is_some());
        let payload = record
            .exception_payload
            .expect("exception_payload should be Some after with_exception");
        assert_eq!(payload.type_name, "ValueError");
        assert_eq!(payload.message, "bad input");
    }

    #[rstest]
    fn with_stack_attaches_payload() {
        let frames = vec![StackFrame::new("test.py", 10, "main")];
        let stack = StackTracePayload::new(frames);
        let record = FemtoLogRecord::new("test", FemtoLevel::Debug, "trace").with_stack(stack);

        assert!(record.stack_payload.is_some());
        let payload = record
            .stack_payload
            .expect("stack_payload should be Some after with_stack");
        assert_eq!(payload.frames.len(), 1);
        assert_eq!(payload.frames[0].function, "main");
    }

    #[rstest]
    fn record_can_have_both_payloads() {
        let exc = ExceptionPayload::new("RuntimeError", "oops");
        let stack = StackTracePayload::new(vec![StackFrame::new("a.py", 1, "f")]);

        let record = FemtoLogRecord::new("test", FemtoLevel::Error, "error")
            .with_exception(exc)
            .with_stack(stack);

        assert!(record.exception_payload.is_some());
        assert!(record.stack_payload.is_some());
    }
}
