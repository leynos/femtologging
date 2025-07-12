use crate::log_record::FemtoLogRecord;
use pyo3::prelude::*;

/// Trait implemented by all log handlers.
///
/// `FemtoHandler` is `Send + Sync` so it can be safely called from multiple
/// threads by reference. Each implementation forwards the record to its own
/// consumer thread without blocking the caller.
pub trait FemtoHandlerTrait: Send + Sync {
    /// Dispatch a log record for handling.
    fn handle(&self, record: FemtoLogRecord);

    /// Flush any pending log records.
    ///
    /// Returning `true` signals the flush completed successfully. Implementations
    /// may return `false` when the handler has been closed or if the flush
    /// command could not be processed.
    fn flush(&self) -> bool {
        // Default to a no-op flush for handlers that do not buffer writes.
        true
    }
}

/// Base Python class for handlers. Methods do nothing by default.
#[pyclass(name = "FemtoHandler", subclass)]
#[derive(Default)]
pub struct FemtoHandler;

#[pymethods]
impl FemtoHandler {
    #[new]
    fn py_new() -> Self {
        Self
    }
}

impl FemtoHandlerTrait for FemtoHandler {
    fn handle(&self, _record: FemtoLogRecord) {}

    fn flush(&self) -> bool {
        true
    }
}
