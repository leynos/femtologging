//! Serialization helpers for HTTP payloads.
//!
//! Provides URL-encoded form data (CPython `logging.HTTPHandler` default) and
//! JSON serialization formats for log records.
//!
//! The URL encoding uses `+` for spaces to match CPython's `urllib.parse.urlencode`
//! behaviour (which uses `quote_plus` internally).

use std::collections::{BTreeMap, HashSet};
use std::io;
use std::thread::ThreadId;

use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde::Serialize;
use serde::ser::{SerializeMap, Serializer};

use crate::exception_schema::{ExceptionPayload, StackTracePayload};
use crate::log_record::FemtoLogRecord;

/// Characters to percent-encode in URL query values (excluding space).
///
/// This encodes all control characters plus characters with special meaning in
/// URLs (query separators, reserved characters), while leaving unreserved
/// characters (alphanumeric, `-`, `_`, `.`, `~`) as-is per RFC 3986.
///
/// Space is handled separately by [`url_encode`] which maps it directly to `+`
/// during iteration, avoiding a second pass over the encoded string.
const QUERY_ENCODE_SET_NO_SPACE: &AsciiSet = &CONTROLS
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}')
    .add(b'\'');

/// Zero-copy serializable record for HTTP payloads.
///
/// This struct borrows from the original record to avoid allocations for string
/// fields during serialization. The `level` field holds a `&'static str` from
/// `FemtoLevel::as_str()`, enabling zero-allocation level serialization.
/// The `thread_id` field stores the raw `ThreadId` and is formatted only during
/// serialization, avoiding per-record String allocations.
struct HttpSerializableRecord<'a> {
    name: &'a str,
    levelname: &'static str,
    msg: &'a str,
    created: f64,
    filename: &'a str,
    lineno: u32,
    module: &'a str,
    thread_id: ThreadId,
    thread_name: Option<&'a str>,
    key_values: &'a BTreeMap<String, String>,
    exc_info: Option<&'a ExceptionPayload>,
    stack_info: Option<&'a StackTracePayload>,
}

impl HttpSerializableRecord<'_> {
    /// Count the total number of fields that will be serialized.
    fn count_fields(&self) -> usize {
        8 + usize::from(self.thread_name.is_some())
            + self.key_values.len()
            + usize::from(self.exc_info.is_some())
            + usize::from(self.stack_info.is_some())
    }

    /// Format the thread ID as a debug string for serialization.
    fn thread_string(&self) -> String {
        format!("{:?}", self.thread_id)
    }
}

impl<'a> From<&'a FemtoLogRecord> for HttpSerializableRecord<'a> {
    fn from(record: &'a FemtoLogRecord) -> Self {
        let metadata = record.metadata();
        let created = metadata
            .timestamp
            .duration_since(std::time::UNIX_EPOCH)
            .map(|dur| dur.as_secs_f64())
            .unwrap_or_default();

        Self {
            name: record.logger(),
            levelname: record.level_str(),
            msg: record.message(),
            created,
            filename: &metadata.filename,
            lineno: metadata.line_number,
            module: &metadata.module_path,
            thread_id: metadata.thread_id,
            thread_name: metadata.thread_name.as_deref(),
            key_values: &metadata.key_values,
            exc_info: record.exception_payload(),
            stack_info: record.stack_payload(),
        }
    }
}

impl Serialize for HttpSerializableRecord<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.count_fields()))?;
        map.serialize_entry("name", self.name)?;
        map.serialize_entry("levelname", self.levelname)?;
        map.serialize_entry("msg", self.msg)?;
        map.serialize_entry("created", &self.created)?;
        map.serialize_entry("filename", self.filename)?;
        map.serialize_entry("lineno", &self.lineno)?;
        map.serialize_entry("module", self.module)?;
        map.serialize_entry("thread", &self.thread_string())?;
        if let Some(name) = self.thread_name {
            map.serialize_entry("threadName", name)?;
        }
        for (k, v) in self.key_values {
            map.serialize_entry(k, v)?;
        }
        if let Some(exc) = self.exc_info {
            map.serialize_entry("exc_info", exc)?;
        }
        if let Some(stack) = self.stack_info {
            map.serialize_entry("stack_info", stack)?;
        }
        map.end()
    }
}

/// Wrapper for filtered serialization with zero-copy where possible.
struct FilteredRecord<'a> {
    record: HttpSerializableRecord<'a>,
    fields: &'a HashSet<&'a str>,
}

/// Base field names for counting and iteration.
const BASE_FIELDS: [&str; 8] = [
    "name",
    "levelname",
    "msg",
    "created",
    "filename",
    "lineno",
    "module",
    "thread",
];

impl<'a> FilteredRecord<'a> {
    /// Count the total number of fields that will be serialized.
    fn count_fields(&self) -> usize {
        let r = &self.record;
        let f = self.fields;

        let base_count = BASE_FIELDS.iter().filter(|&&k| f.contains(k)).count();
        let optional_count = usize::from(f.contains("threadName") && r.thread_name.is_some())
            + usize::from(f.contains("exc_info") && r.exc_info.is_some())
            + usize::from(f.contains("stack_info") && r.stack_info.is_some())
            + r.key_values
                .keys()
                .filter(|k| f.contains(k.as_str()))
                .count();

        base_count + optional_count
    }

    /// Serialize key-value pairs that match the filter.
    fn serialize_key_values<S>(
        &self,
        map: &mut <S as Serializer>::SerializeMap,
    ) -> Result<(), S::Error>
    where
        S: Serializer,
    {
        for (k, v) in self.record.key_values {
            if self.fields.contains(k.as_str()) {
                map.serialize_entry(k, v)?;
            }
        }
        Ok(())
    }

    /// Serialize optional fields (threadName, key_values, exc_info, stack_info).
    fn serialize_optional_fields<S>(
        &self,
        map: &mut <S as Serializer>::SerializeMap,
    ) -> Result<(), S::Error>
    where
        S: Serializer,
    {
        let r = &self.record;
        let f = self.fields;

        if f.contains("threadName")
            && let Some(name) = r.thread_name
        {
            map.serialize_entry("threadName", name)?;
        }
        self.serialize_key_values::<S>(map)?;
        if f.contains("exc_info")
            && let Some(exc) = r.exc_info
        {
            map.serialize_entry("exc_info", exc)?;
        }
        if f.contains("stack_info")
            && let Some(stack) = r.stack_info
        {
            map.serialize_entry("stack_info", stack)?;
        }
        Ok(())
    }
}

impl Serialize for FilteredRecord<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let r = &self.record;
        let f = self.fields;

        let mut map = serializer.serialize_map(Some(self.count_fields()))?;

        macro_rules! emit {
            ($key:literal, $val:expr) => {
                if f.contains($key) {
                    map.serialize_entry($key, $val)?;
                }
            };
        }

        emit!("name", r.name);
        emit!("levelname", r.levelname);
        emit!("msg", r.msg);
        emit!("created", &r.created);
        emit!("filename", r.filename);
        emit!("lineno", &r.lineno);
        emit!("module", r.module);
        if f.contains("thread") {
            map.serialize_entry("thread", &r.thread_string())?;
        }

        self.serialize_optional_fields::<S>(&mut map)?;
        map.end()
    }
}

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
    emit_string_field(pairs, "thread", &r.thread_string(), has);
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

/// URL-encode a string using `+` for spaces (CPython `urlencode` parity).
///
/// This matches the behaviour of `urllib.parse.urlencode`, which uses
/// `quote_plus` internally and encodes spaces as `+` rather than `%20`.
///
/// Spaces are mapped to `+` directly during encoding (single pass), rather than
/// encoding to `%20` and then replacing in a second pass.
fn url_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut first = true;
    for chunk in s.split(' ') {
        if !first {
            result.push('+');
        }
        first = false;
        result.push_str(&utf8_percent_encode(chunk, QUERY_ENCODE_SET_NO_SPACE).to_string());
    }
    result
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

    #[test]
    fn url_encode_special_chars() {
        assert_eq!(url_encode("hello world"), "hello+world");
        assert_eq!(url_encode("a=b&c=d"), "a%3Db%26c%3Dd");
        assert_eq!(url_encode("test_value-123.txt"), "test_value-123.txt");
    }

    #[test]
    fn url_encode_edge_cases() {
        // Empty string
        assert_eq!(url_encode(""), "");
        // Consecutive spaces
        assert_eq!(url_encode("a  b"), "a++b");
        assert_eq!(url_encode("a   b"), "a+++b");
        // Leading spaces
        assert_eq!(url_encode(" hello"), "+hello");
        assert_eq!(url_encode("  hello"), "++hello");
        // Trailing spaces
        assert_eq!(url_encode("hello "), "hello+");
        assert_eq!(url_encode("hello  "), "hello++");
        // Only spaces
        assert_eq!(url_encode(" "), "+");
        assert_eq!(url_encode("  "), "++");
        assert_eq!(url_encode("   "), "+++");
    }
}
