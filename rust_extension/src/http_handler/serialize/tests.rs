//! Tests for HTTP record serialization.

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
    let encoded = serialize_url_encoded(&test_record, None).expect("serialize");
    assert!(encoded.contains("name=test.logger"));
    assert!(encoded.contains("levelname=INFO"));
    assert!(encoded.contains("msg=Hello+World"));
    assert!(encoded.contains("lineno=42"));
}

#[rstest]
fn json_contains_expected_fields(test_record: FemtoLogRecord) {
    let json = serialize_json(&test_record, None).expect("serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
    assert_eq!(parsed["name"], "test.logger");
    assert_eq!(parsed["levelname"], "INFO");
    assert_eq!(parsed["msg"], "Hello World");
    assert_eq!(parsed["lineno"], 42);
}

#[rstest]
fn field_filter_limits_output(test_record: FemtoLogRecord) {
    let fields = vec!["name".into(), "msg".into()];
    let json = serialize_json(&test_record, Some(&fields)).expect("serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
    assert_eq!(parsed["name"], "test.logger");
    assert_eq!(parsed["msg"], "Hello World");
    assert!(parsed.get("levelname").is_none());
    assert!(parsed.get("lineno").is_none());
}

/// Reserved metadata names and one legitimate custom field.
const COLLIDING_KEY_VALUES: &[(&str, &str)] = &[
    ("name", "spoofed.logger"),
    ("levelname", "CRITICAL"),
    ("msg", "spoofed message"),
    ("filename", "spoofed.rs"),
    ("lineno", "999"),
    ("module", "spoofed.module"),
    ("threadName", "spoofed-thread"),
    ("exc_info", "spoofed-exc"),
    ("stack_info", "spoofed-stack"),
    ("logger", "spoofed-logger"),
    ("level", "CRITICAL"),
    ("message", "spoofed-message"),
    ("correlation_id", "abc-123"),
];

/// Build a record whose metadata bypasses structured-field admission.
fn record_with_key_values(key_values: &[(&str, &str)]) -> FemtoLogRecord {
    let metadata = RecordMetadata {
        module_path: "test.module".into(),
        filename: "test.rs".into(),
        line_number: 42,
        key_values: key_values
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect(),
        ..RecordMetadata::default()
    };
    FemtoLogRecord::with_metadata("test.logger", FemtoLevel::Info, "Hello World", metadata)
}

/// Build a collision record with a known authoritative thread name.
fn record_with_authoritative_thread(key_values: &[(&str, &str)]) -> FemtoLogRecord {
    let mut record = record_with_key_values(key_values);
    record.metadata_mut().thread_name = Some("authoritative-thread".to_owned());
    record
}

#[rstest]
fn json_omits_reserved_metadata_and_keeps_authoritative_fields() {
    let record = record_with_authoritative_thread(COLLIDING_KEY_VALUES);

    let json = serialize_json(&record, None).expect("serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");

    assert_eq!(parsed["name"], "test.logger");
    assert_eq!(parsed["levelname"], "INFO");
    assert_eq!(parsed["msg"], "Hello World");
    assert_eq!(parsed["filename"], "test.rs");
    assert_eq!(parsed["lineno"], 42);
    assert_eq!(parsed["module"], "test.module");
    assert_eq!(parsed["threadName"], "authoritative-thread");
    assert_eq!(parsed["correlation_id"], "abc-123");

    for key in [
        "name",
        "levelname",
        "msg",
        "filename",
        "lineno",
        "module",
        "threadName",
        "correlation_id",
    ] {
        assert_eq!(
            json.matches(&format!("\"{key}\"")).count(),
            1,
            "field '{key}' should appear exactly once"
        );
    }
    for key in ["exc_info", "logger", "level", "message"] {
        assert_eq!(
            json.matches(&format!("\"{key}\"")).count(),
            0,
            "reserved field '{key}' must not be emitted from metadata"
        );
    }
}

#[rstest]
fn json_omits_reserved_metadata_even_when_builtin_field_is_absent() {
    let mut record = record_with_key_values(&[
        ("threadName", "spoofed-thread"),
        ("exc_info", "spoofed-exc"),
        ("stack_info", "spoofed-stack"),
    ]);
    record.metadata_mut().thread_name = None;

    let json = serialize_json(&record, None).expect("serialize");

    for key in ["threadName", "exc_info", "stack_info"] {
        assert_eq!(
            json.matches(&format!("\"{key}\"")).count(),
            0,
            "reserved field '{key}' must stay omitted when the built-in is absent"
        );
    }
}

#[rstest]
fn filtered_json_uses_authoritative_values_for_reserved_fields() {
    let record = record_with_authoritative_thread(COLLIDING_KEY_VALUES);
    let fields = vec![
        "name".into(),
        "levelname".into(),
        "msg".into(),
        "module".into(),
        "correlation_id".into(),
    ];

    let json = serialize_json(&record, Some(&fields)).expect("serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");

    assert_eq!(parsed["name"], "test.logger");
    assert_eq!(parsed["levelname"], "INFO");
    assert_eq!(parsed["msg"], "Hello World");
    assert_eq!(parsed["module"], "test.module");
    assert_eq!(parsed["correlation_id"], "abc-123");

    for key in ["name", "levelname", "msg", "module", "correlation_id"] {
        assert_eq!(
            json.matches(&format!("\"{key}\"")).count(),
            1,
            "filtered field '{key}' should appear exactly once"
        );
    }
}

/// Split URL-encoded output into its key-value pairs.
fn url_pairs(encoded: &str) -> Vec<(&str, &str)> {
    encoded
        .split('&')
        .map(|pair| pair.split_once('=').expect("pair should contain '='"))
        .collect()
}

#[rstest]
fn url_encoded_omits_reserved_metadata_and_keeps_unique_keys() {
    let record = record_with_authoritative_thread(COLLIDING_KEY_VALUES);

    let encoded = serialize_url_encoded(&record, None).expect("serialize");
    let pairs = url_pairs(&encoded);
    let keys: std::collections::HashSet<&str> = pairs.iter().map(|(key, _)| *key).collect();

    assert_eq!(keys.len(), pairs.len(), "URL-encoded keys must be unique");
    assert!(pairs.contains(&("name", "test.logger")));
    assert!(pairs.contains(&("levelname", "INFO")));
    assert!(pairs.contains(&("msg", "Hello+World")));
    assert!(pairs.contains(&("filename", "test.rs")));
    assert!(pairs.contains(&("lineno", "42")));
    assert!(pairs.contains(&("module", "test.module")));
    assert!(pairs.contains(&("threadName", "authoritative-thread")));
    assert!(pairs.contains(&("correlation_id", "abc-123")));
    for key in ["exc_info", "logger", "level", "message"] {
        assert!(
            !keys.contains(key),
            "reserved field '{key}' must not be emitted from metadata"
        );
    }
}

#[rstest]
fn filtered_url_encoded_uses_authoritative_values_for_reserved_fields() {
    let record = record_with_authoritative_thread(COLLIDING_KEY_VALUES);
    let fields = vec![
        "name".into(),
        "levelname".into(),
        "msg".into(),
        "module".into(),
        "correlation_id".into(),
    ];

    let encoded = serialize_url_encoded(&record, Some(&fields)).expect("serialize");
    let pairs = url_pairs(&encoded);
    let keys: std::collections::HashSet<&str> = pairs.iter().map(|(key, _)| *key).collect();

    assert_eq!(keys.len(), pairs.len(), "URL-encoded keys must be unique");
    assert_eq!(pairs.len(), fields.len());
    assert!(pairs.contains(&("name", "test.logger")));
    assert!(pairs.contains(&("levelname", "INFO")));
    assert!(pairs.contains(&("msg", "Hello+World")));
    assert!(pairs.contains(&("module", "test.module")));
    assert!(pairs.contains(&("correlation_id", "abc-123")));
}

#[rstest]
fn legitimate_custom_fields_remain_flat_and_string_valued() {
    let record =
        record_with_authoritative_thread(&[("correlation_id", "abc-123"), ("request_id", "req-1")]);

    let json = serialize_json(&record, None).expect("serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
    assert_eq!(parsed["correlation_id"], "abc-123");
    assert_eq!(parsed["request_id"], "req-1");

    let encoded = serialize_url_encoded(&record, None).expect("serialize");
    let pairs = url_pairs(&encoded);
    assert!(pairs.ends_with(&[("correlation_id", "abc-123"), ("request_id", "req-1"),]));
}
