//! Reserved record and metadata names that structured fields must not use.
//!
//! The sets are shared by context admission, callback enrichment, and HTTP
//! serialization so every path agrees on which names are built-in fields.

/// Stdlib `logging.LogRecord` attribute names reserved for built-in fields.
const RESERVED_STD_RECORD_KEYS: &[&str] = &[
    "name",
    "msg",
    "args",
    "levelname",
    "levelno",
    "pathname",
    "filename",
    "module",
    "exc_info",
    "exc_text",
    "stack_info",
    "lineno",
    "funcName",
    "created",
    "msecs",
    "relativeCreated",
    "thread",
    "threadName",
    "process",
    "processName",
    "message",
    "asctime",
    "taskName",
];

/// Femtologging-specific record and metadata names reserved for built-in fields.
const RESERVED_FEMTO_KEYS: &[&str] = &[
    "logger",
    "level",
    "metadata",
    "module_path",
    "line_number",
    "timestamp",
    "thread_id",
    "thread_name",
    "key_values",
];

/// Report whether `key` collides with a built-in record or metadata field.
///
/// Matching is exact and case-sensitive: `"Name"` and `"logger_name"` are
/// custom fields, while `"name"` and `"logger"` are reserved.
pub(crate) fn is_reserved_structured_key(key: &str) -> bool {
    RESERVED_STD_RECORD_KEYS.contains(&key) || RESERVED_FEMTO_KEYS.contains(&key)
}
