//! Serialization helpers for HTTP payloads.
//!
//! Provides URL-encoded form data (CPython `logging.HTTPHandler` default) and
//! JSON serialization formats for log records.
//!
//! The URL encoding uses `+` for spaces to match CPython's `urllib.parse.urlencode`
//! behaviour (which uses `quote_plus` internally).

use std::collections::HashSet;
use std::io;

use serde::Serialize;

use super::filtered::FilteredRecord;
use super::record::HttpSerializableRecord;
use super::url_encoding::url_encode;
use crate::log_record::FemtoLogRecord;

/// Emit a numeric field as URL-encoded key=value pair if included by the filter.
fn emit_numeric_field(
    pairs: &mut Vec<String>,
    key: &str,
    value: impl std::fmt::Display,
    has: &impl Fn(&str) -> bool,
) {
    if has(key) {
        pairs.push(format!("{key}={value}"));
    }
}

/// Emit a JSON-serialised optional field as URL-encoded key=value pair.
fn emit_json_field<T: Serialize>(
    pairs: &mut Vec<String>,
    key: &str,
    value: Option<&T>,
    has: &impl Fn(&str) -> bool,
) -> io::Result<()> {
    if has(key)
        && let Some(v) = value
    {
        let json = serde_json::to_string(v).map_err(io::Error::other)?;
        pairs.push(format!("{}={}", url_encode(key), url_encode(&json)));
    }
    Ok(())
}

/// Emit key-value pairs as URL-encoded pairs if included by the filter.
fn emit_key_values<'a>(
    pairs: &mut Vec<String>,
    key_values: impl Iterator<Item = (&'a String, &'a String)>,
    has: &impl Fn(&str) -> bool,
) {
    for (k, v) in key_values {
        if has(k) {
            pairs.push(format!("{}={}", url_encode(k), url_encode(v)));
        }
    }
}

/// Emit a string field as URL-encoded key=value pair if included by the filter.
fn emit_string_field(pairs: &mut Vec<String>, key: &str, value: &str, has: &impl Fn(&str) -> bool) {
    if has(key) {
        pairs.push(format!("{}={}", url_encode(key), url_encode(value)));
    }
}

/// Emit an optional string field as URL-encoded key=value pair.
fn emit_optional_string_field(
    pairs: &mut Vec<String>,
    key: &str,
    value: Option<&str>,
    has: &impl Fn(&str) -> bool,
) {
    if has(key)
        && let Some(v) = value
    {
        pairs.push(format!("{}={}", url_encode(key), url_encode(v)));
    }
}

/// Emit all fields from an `HttpSerializableRecord` as URL-encoded pairs.
fn emit_all_fields(
    pairs: &mut Vec<String>,
    r: &HttpSerializableRecord<'_>,
    has: &impl Fn(&str) -> bool,
) -> io::Result<()> {
    emit_string_field(pairs, "name", r.name, has);
    emit_string_field(pairs, "levelname", r.levelname, has);
    emit_string_field(pairs, "msg", r.msg, has);
    emit_numeric_field(pairs, "created", r.created, has);
    emit_string_field(pairs, "filename", r.filename, has);
    emit_numeric_field(pairs, "lineno", r.lineno, has);
    emit_string_field(pairs, "module", r.module, has);
    if has("thread") {
        pairs.push(format!("thread={:?}", r.thread_id));
    }
    emit_optional_string_field(pairs, "threadName", r.thread_name, has);
    emit_key_values(pairs, r.key_values.iter(), has);
    emit_json_field(pairs, "exc_info", r.exc_info, has)?;
    emit_json_field(pairs, "stack_info", r.stack_info, has)?;
    Ok(())
}

/// Serialise a record to URL-encoded form data (CPython parity).
///
/// This produces output compatible with `urllib.parse.urlencode(record.__dict__)`,
/// using `+` for spaces as CPython's `urlencode` does by default.
///
/// # Arguments
///
/// * `record` - The log record to serialise.
/// * `fields` - Optional list of field names to include. If `None`, all fields
///   are included.
///
/// # Returns
///
/// A URL-encoded string representation of the record fields, or an
/// [`io::Error`] if JSON serialization of exception/stack payloads fails.
///
/// # Errors
///
/// Returns an error if JSON serialization of `exc_info` or `stack_info`
/// payloads fails.
pub fn serialise_url_encoded(
    record: &FemtoLogRecord,
    fields: Option<&[String]>,
) -> io::Result<String> {
    let r = HttpSerializableRecord::from(record);
    let filter: Option<HashSet<&str>> = fields.map(|f| f.iter().map(String::as_str).collect());
    let has = |name: &str| filter.as_ref().is_none_or(|f| f.contains(name));

    let mut pairs = Vec::new();
    emit_all_fields(&mut pairs, &r, &has)?;
    Ok(pairs.join("&"))
}

/// Serialise a record to JSON.
///
/// Uses zero-copy serialization where possible to avoid allocations.
/// The `levelname` field is serialized directly from `&'static str`.
///
/// # Arguments
///
/// * `record` - The log record to serialise.
/// * `fields` - Optional list of field names to include. If `None`, all fields
///   are included.
///
/// # Returns
///
/// A JSON string representation of the record fields, or an [`io::Error`] if
/// serialization fails.
///
/// # Errors
///
/// Returns an error if JSON serialization fails.
pub fn serialise_json(record: &FemtoLogRecord, fields: Option<&[String]>) -> io::Result<String> {
    let serializable = HttpSerializableRecord::from(record);

    match fields {
        Some(f) => {
            let field_set: HashSet<&str> = f.iter().map(String::as_str).collect();
            let filtered = FilteredRecord {
                record: serializable,
                fields: &field_set,
            };
            serde_json::to_string(&filtered).map_err(io::Error::other)
        }
        None => serde_json::to_string(&serializable).map_err(io::Error::other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::FemtoLevel;
    use crate::log_record::RecordMetadata;
    use rstest::{fixture, rstest};

    #[fixture]
    fn test_record() -> FemtoLogRecord {
        let metadata = RecordMetadata {
            module_path: "test.module".into(),
            filename: "test.rs".into(),
            line_number: 42,
            ..RecordMetadata::default()
        };
        FemtoLogRecord::with_metadata("test.logger", FemtoLevel::Info, "Hello World", metadata)
    }

    #[rstest]
    fn url_encoded_contains_expected_fields(test_record: FemtoLogRecord) {
        let encoded = serialise_url_encoded(&test_record, None).expect("serialise");
        assert!(encoded.contains("name=test.logger"));
        assert!(encoded.contains("levelname=INFO"));
        assert!(encoded.contains("msg=Hello+World"));
        assert!(encoded.contains("lineno=42"));
    }

    #[rstest]
    fn json_contains_expected_fields(test_record: FemtoLogRecord) {
        let json = serialise_json(&test_record, None).expect("serialise");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed["name"], "test.logger");
        assert_eq!(parsed["levelname"], "INFO");
        assert_eq!(parsed["msg"], "Hello World");
        assert_eq!(parsed["lineno"], 42);
    }

    #[rstest]
    fn field_filter_limits_output(test_record: FemtoLogRecord) {
        let fields = vec!["name".into(), "msg".into()];
        let json = serialise_json(&test_record, Some(&fields)).expect("serialise");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed["name"], "test.logger");
        assert_eq!(parsed["msg"], "Hello World");
        assert!(parsed.get("levelname").is_none());
        assert!(parsed.get("lineno").is_none());
    }
}
