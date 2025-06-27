use crate::log_record::FemtoLogRecord;

/// Trait for formatting log records into strings.
///
/// Implementors must be thread-safe (`Send + Sync`) so formatters can be
/// shared across threads in a logging system.
pub trait FemtoFormatter: Send + Sync {
    /// Format a log record into a string representation.
    fn format(&self, record: &FemtoLogRecord) -> String;
}

#[derive(Copy, Clone, Debug)]
pub struct DefaultFormatter;

impl FemtoFormatter for DefaultFormatter {
    fn format(&self, record: &FemtoLogRecord) -> String {
        format!("{}: {} - {}", record.logger, record.level, record.message)
    }
}
