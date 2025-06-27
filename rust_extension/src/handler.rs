use crate::log_record::FemtoLogRecord;

/// Trait implemented by all log handlers.
///
/// `FemtoHandler` is `Send + Sync` so it can be safely called from multiple
/// threads by reference. Each implementation forwards the record to its own
/// consumer thread without blocking the caller.
pub trait FemtoHandler: Send + Sync {
    /// Dispatch a log record for handling.
    fn handle(&self, record: FemtoLogRecord);
}
