//! Serialization helpers for HTTP payloads.
//!
//! Provides URL-encoded form data (CPython `logging.HTTPHandler` default) and
//! JSON serialization formats for log records.

use std::collections::BTreeMap;
use std::io;

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
        serde_json::Value::String(record.level.clone()),
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
    map
}

fn filter_fields(
    full_map: BTreeMap<String, serde_json::Value>,
    fields: &[String],
) -> BTreeMap<String, serde_json::Value> {
    full_map
        .into_iter()
        .filter(|(k, _)| fields.iter().any(|f| f == k))
        .collect()
}

/// Serialise a record to URL-encoded form data (CPython parity).
///
/// This produces output compatible with `urllib.parse.urlencode(record.__dict__)`.
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

/// URL-encode a string following RFC 3986.
fn url_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                result.push('%');
                result.push(to_hex_upper(byte >> 4));
                result.push(to_hex_upper(byte & 0x0F));
            }
        }
    }
    result
}

fn to_hex_upper(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        10..=15 => (b'A' + nibble - 10) as char,
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_record::RecordMetadata;

    fn test_record() -> FemtoLogRecord {
        FemtoLogRecord {
            logger: "test.logger".into(),
            level: "INFO".into(),
            parsed_level: None,
            message: "Hello World".into(),
            metadata: RecordMetadata {
                module_path: "test.module".into(),
                filename: "test.rs".into(),
                line_number: 42,
                ..Default::default()
            },
        }
    }

    #[test]
    fn url_encoded_contains_expected_fields() {
        let record = test_record();
        let encoded = serialise_url_encoded(&record, None).expect("serialise");
        assert!(encoded.contains("name=test.logger"));
        assert!(encoded.contains("levelname=INFO"));
        assert!(encoded.contains("msg=Hello%20World"));
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
        assert_eq!(url_encode("hello world"), "hello%20world");
        assert_eq!(url_encode("a=b&c=d"), "a%3Db%26c%3Dd");
        assert_eq!(url_encode("test_value-123.txt"), "test_value-123.txt");
    }
}
