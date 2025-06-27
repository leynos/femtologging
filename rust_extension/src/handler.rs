use crate::log_record::FemtoLogRecord;

/// Trait implemented by all log handlers.
///
/// `FemtoHandler` is `Send` so it can be invoked from multiple threads.
/// Each implementation forwards the record to its own consumer thread
/// without blocking the caller.
pub trait FemtoHandler: Send {
    /// Dispatch a log record for handling.
    fn handle(&self, record: FemtoLogRecord);
}
