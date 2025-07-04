use std::collections::BTreeMap;
/// Represents a single log event with associated metadata.
///
/// `FemtoLogRecord` captures not only the textual message but also contextual
/// information about where and when the log was created. This data can later be
/// used by formatters and handlers. The structure remains simple so that
/// creating a record on the logging hot path is cheap.
use std::fmt;
use std::thread::{self, ThreadId};
use std::time::SystemTime;

#[derive(Clone, Debug)]
pub struct FemtoLogRecord {
    /// Name of the logger that created this record.
    pub logger: String,
    /// The log level as a string (e.g. "INFO" or "ERROR").
    pub level: String,
    /// The log message content.
    pub message: String,
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

impl FemtoLogRecord {
    /// Construct a new log record from logger `name`, `level`, and `message`.
    pub fn new(logger: &str, level: &str, message: &str) -> Self {
        Self {
            logger: logger.to_owned(),
            level: level.to_owned(),
            message: message.to_owned(),
            module_path: String::new(),
            filename: String::new(),
            line_number: 0,
            timestamp: SystemTime::now(),
            thread_id: thread::current().id(),
            thread_name: thread::current().name().map(|n| n.to_string()),
            key_values: BTreeMap::new(),
        }
    }

    /// Construct a log record with explicit source location and key-values.
    pub fn with_metadata(
        logger: &str,
        level: &str,
        message: &str,
        module_path: &str,
        filename: &str,
        line_number: u32,
        key_values: BTreeMap<String, String>,
    ) -> Self {
        Self {
            logger: logger.to_owned(),
            level: level.to_owned(),
            message: message.to_owned(),
            module_path: module_path.to_owned(),
            filename: filename.to_owned(),
            line_number,
            timestamp: SystemTime::now(),
            thread_id: thread::current().id(),
            thread_name: thread::current().name().map(|n| n.to_string()),
            key_values,
        }
    }
}

impl fmt::Display for FemtoLogRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} - {}", self.level, self.message)
    }
}
