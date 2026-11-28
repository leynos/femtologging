//! Public producer entry points that build a record from caller arguments.
//!
//! Each method here starts the same funnel: a disabled logger is refused and
//! counted, the record is built with this logger's name, and the shared record
//! path applies the level gate, filters, formatting and dispatch. The prebuilt
//! path for a caller-assembled record stays in the parent module, beside the
//! context-drop diagnostic it reports.

use crate::level::FemtoLevel;
use crate::log_record::{FemtoLogRecord, RecordMetadata};

use super::super::FemtoLogger;

impl FemtoLogger {
    /// Log a message at the given level (Rust-only API).
    ///
    /// This method is a simplified version of the PyO3-exposed `log` method
    /// for pure Rust callers. It does not support `exc_info` or `stack_info`.
    pub fn log(&self, level: FemtoLevel, message: &str) -> Option<String> {
        if self.refuse_if_disabled() {
            return None;
        }
        let record = FemtoLogRecord::new(&self.name, level, message);
        self.log_record(record)
    }

    /// Log a message with explicit source location metadata.
    ///
    /// Used by the `femtolog_info!` family of macros and the Python
    /// convenience functions to attach caller-captured source information
    /// (filename, line number, module path) to the record before filtering
    /// and dispatch.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use femtologging_rs::{FemtoLevel, FemtoLogger, RecordMetadata};
    ///
    /// let logger = FemtoLogger::new("example".into());
    /// let metadata = RecordMetadata {
    ///     filename: "main.rs".into(),
    ///     line_number: 42,
    ///     module_path: "example".into(),
    ///     ..Default::default()
    /// };
    /// logger.log_with_metadata(FemtoLevel::Info, "hello", metadata);
    /// ```
    pub fn log_with_metadata(
        &self,
        level: FemtoLevel,
        message: &str,
        metadata: RecordMetadata,
    ) -> Option<String> {
        self.log_prebuilt(FemtoLogRecord::with_metadata(
            &self.name, level, message, metadata,
        ))
    }

    /// Log a record whose context was captured by the Python caller.
    ///
    /// Python task-local context is already present in `metadata`; Rust
    /// thread-local context is deliberately not merged a second time.
    #[cfg(feature = "python")]
    pub(crate) fn log_with_explicit_metadata(
        &self,
        level: FemtoLevel,
        message: &str,
        metadata: RecordMetadata,
    ) -> Option<String> {
        if self.refuse_if_disabled() || !self.is_enabled_for(level) {
            return None;
        }
        let record = FemtoLogRecord::with_metadata(&self.name, level, message, metadata);
        self.log_record(record)
    }
}
