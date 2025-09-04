//! Filtering trait for log records.
//!
//! Defines [`FemtoFilter`] which allows loggers and handlers to
//! decide whether a [`FemtoLogRecord`] should be processed.

use crate::log_record::FemtoLogRecord;

/// Trait implemented by all log filters.
///
/// Filters are `Send + Sync` so they can be shared across threads.
pub trait FemtoFilter: Send + Sync {
    /// Return `true` if `record` should be processed.
    fn should_log(&self, record: &FemtoLogRecord) -> bool;
}
