/// Represents a single log event with level and message data.
///
/// This struct is intentionally minimal for now. Additional fields such as a
/// timestamp may be added as functionality grows.
use std::fmt;

#[derive(Clone, Debug)]
pub struct FemtoLogRecord {
    /// Name of the logger that created this record.
    pub logger: String,
    /// The log level as a string (e.g. "INFO" or "ERROR").
    pub level: String,
    /// The log message content.
    pub message: String,
}

impl FemtoLogRecord {
    /// Construct a new log record from logger `name`, `level`, and `message`.
    pub fn new(logger: &str, level: &str, message: &str) -> Self {
        Self {
            logger: logger.to_owned(),
            level: level.to_owned(),
            message: message.to_owned(),
        }
    }
}

impl fmt::Display for FemtoLogRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} - {}", self.level, self.message)
    }
}
