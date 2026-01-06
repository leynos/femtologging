//! Serialization helpers for HTTP payloads.
//!
//! Provides URL-encoded form data (CPython `logging.HTTPHandler` default) and
//! JSON serialization formats for log records.
//!
//! The URL encoding uses `+` for spaces to match CPython's `urllib.parse.urlencode`
//! behaviour (which uses `quote_plus` internally).

use std::collections::{BTreeMap, HashSet};
use std::io;

use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};

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

use crate::log_record::FemtoLogRecord;

fn build_full_map(record: &FemtoLogRecord) -> BTreeMap<String, serde_json::Value> {
    let timestamp = record
        .metadata
        .timestamp
        .duration_since(std::time::UNIX_EPOCH)
        .map(|dur| dur.as_secs_f64())
        .unwrap_or_default();

    let mut map = BTreeMap::new();
    map.insert(
        "name".into(),
        serde_json::Value::String(record.logger.clone()),
    );
    map.insert(
        "levelname".into(),
        serde_json::Value::String(record.level_str().to_owned()),
    );
    map.insert(
        "msg".into(),
        serde_json::Value::String(record.message.clone()),
    );
    map.insert(
        "created".into(),
        serde_json::Value::Number(
            serde_json::Number::from_f64(timestamp).unwrap_or_else(|| serde_json::Number::from(0)),
        ),
    );
    map.insert(
        "filename".into(),
        serde_json::Value::String(record.metadata.filename.clone()),
    );
    map.insert(
        "lineno".into(),
        serde_json::Value::Number(record.metadata.line_number.into()),
    );
    map.insert(
        "module".into(),
        serde_json::Value::String(record.metadata.module_path.clone()),
    );
    map.insert(
        "thread".into(),
        serde_json::Value::String(format!("{:?}", record.metadata.thread_id)),
    );
    if let Some(ref name) = record.metadata.thread_name {
        map.insert("threadName".into(), serde_json::Value::String(name.clone()));
    }
    for (k, v) in &record.metadata.key_values {
        map.insert(k.clone(), serde_json::Value::String(v.clone()));
    }
    if let Some(ref exc) = record.exception_payload {
        map.insert("exc_info".into(), serde_json::json!(exc));
    }
    if let Some(ref stack) = record.stack_payload {
        map.insert("stack_info".into(), serde_json::json!(stack));
    }
    map
}

fn filter_fields(
    full_map: BTreeMap<String, serde_json::Value>,
    fields: &[String],
) -> BTreeMap<String, serde_json::Value> {
    let field_set: HashSet<&str> = fields.iter().map(String::as_str).collect();
    full_map
        .into_iter()
        .filter(|(k, _)| field_set.contains(k.as_str()))
        .collect()
}

/// Serialise a record to URL-encoded form data (CPython parity).
///
/// This produces output compatible with `urllib.parse.urlencode(record.__dict__)`,
/// using `+` for spaces as CPython's `urlencode` does by default.
pub fn serialise_url_encoded(
    record: &FemtoLogRecord,
    fields: Option<&[String]>,
) -> io::Result<String> {
    let full_map = build_full_map(record);
    let map = match fields {
        Some(f) => filter_fields(full_map, f),
        None => full_map,
    };

    let pairs: Vec<String> = map
        .into_iter()
        .map(|(k, v)| {
            let value_str = match v {
                serde_json::Value::String(s) => s,
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                serde_json::Value::Null => String::new(),
                other => other.to_string(),
            };
            format!("{}={}", url_encode(&k), url_encode(&value_str))
        })
        .collect();

    Ok(pairs.join("&"))
}

/// Serialise a record to JSON.
pub fn serialise_json(record: &FemtoLogRecord, fields: Option<&[String]>) -> io::Result<String> {
    let full_map = build_full_map(record);
    let map = match fields {
        Some(f) => filter_fields(full_map, f),
        None => full_map,
    };

    serde_json::to_string(&map).map_err(io::Error::other)
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

    fn test_record() -> FemtoLogRecord {
        let mut record = FemtoLogRecord::new("test.logger", FemtoLevel::Info, "Hello World");
        record.metadata.module_path = "test.module".into();
        record.metadata.filename = "test.rs".into();
        record.metadata.line_number = 42;
        record
    }

    #[test]
    fn url_encoded_contains_expected_fields() {
        let record = test_record();
        let encoded = serialise_url_encoded(&record, None).expect("serialise");
        assert!(encoded.contains("name=test.logger"));
        assert!(encoded.contains("levelname=INFO"));
        assert!(encoded.contains("msg=Hello+World"));
        assert!(encoded.contains("lineno=42"));
    }

    #[test]
    fn json_contains_expected_fields() {
        let record = test_record();
        let json = serialise_json(&record, None).expect("serialise");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed["name"], "test.logger");
        assert_eq!(parsed["levelname"], "INFO");
        assert_eq!(parsed["msg"], "Hello World");
        assert_eq!(parsed["lineno"], 42);
    }

    #[test]
    fn field_filter_limits_output() {
        let record = test_record();
        let fields = vec!["name".into(), "msg".into()];
        let json = serialise_json(&record, Some(&fields)).expect("serialise");
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
