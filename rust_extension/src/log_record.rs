/// Represents a single log event with level and message data.
///
/// This struct is intentionally minimal for now. Additional fields such as a
/// timestamp may be added as functionality grows.
#[derive(Debug)]
pub struct FemtoLogRecord<'a> {
    /// The log level as a string slice (e.g. "INFO" or "ERROR").
    pub level: &'a str,
    /// The log message content.
    pub message: &'a str,
}
