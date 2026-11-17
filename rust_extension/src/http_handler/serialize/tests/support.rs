//! Assertion helpers shared by the HTTP serialization tests.

use super::*;
use serde_json::Value;

/// Serialize a record to JSON and parse the result.
pub(super) fn parsed_json(record: &FemtoLogRecord, fields: Option<&[String]>) -> Value {
    let json = serialize_json(record, fields).expect("serialize");
    serde_json::from_str(&json).expect("parse")
}

/// Assert each expected field value in a parsed JSON payload.
pub(super) fn assert_json_fields(parsed: &Value, expected: &[(&str, Value)]) {
    for (key, value) in expected {
        assert_eq!(
            parsed[*key], *value,
            "JSON field '{key}' should hold {value}"
        );
    }
}

/// Assert each key appears exactly once in the raw JSON text.
pub(super) fn assert_json_keys_unique(json: &str, keys: &[&str]) {
    for key in keys {
        assert_eq!(
            json.matches(&format!("\"{key}\"")).count(),
            1,
            "field '{key}' should appear exactly once"
        );
    }
}

/// Assert that no key appears in the raw JSON text.
pub(super) fn assert_json_keys_absent(json: &str, keys: &[&str]) {
    for key in keys {
        assert_eq!(
            json.matches(&format!("\"{key}\"")).count(),
            0,
            "reserved field '{key}' must not be emitted from metadata"
        );
    }
}

/// Split URL-encoded output into its key-value pairs.
pub(super) fn url_pairs(encoded: &str) -> Vec<(&str, &str)> {
    encoded
        .split('&')
        .map(|pair| pair.split_once('=').expect("pair should contain '='"))
        .collect()
}

/// Assert each expected pair appears in the URL-encoded output.
pub(super) fn assert_pairs_contain(pairs: &[(&str, &str)], expected: &[(&str, &str)]) {
    for pair in expected {
        assert!(
            pairs.contains(pair),
            "URL-encoded output should contain {pair:?}"
        );
    }
}

/// Assert a key appears exactly once among URL-encoded pairs.
pub(super) fn assert_pair_key_unique(pairs: &[(&str, &str)], key: &str) {
    let count = pairs
        .iter()
        .filter(|(candidate, _)| *candidate == key)
        .count();
    assert_eq!(
        count, 1,
        "URL-encoded key '{key}' should appear exactly once"
    );
}

/// Assert that no reserved key appears among URL-encoded pairs.
pub(super) fn assert_pairs_exclude(pairs: &[(&str, &str)], keys: &[&str]) {
    for key in keys {
        assert!(
            !pairs.iter().any(|(candidate, _)| candidate == key),
            "reserved field '{key}' must not be emitted from metadata"
        );
    }
}

/// Build the field selection passed to a filtered serializer.
pub(super) fn selected_fields(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

/// The authoritative UNIX timestamp serialized for a record.
pub(super) fn created_seconds(record: &FemtoLogRecord) -> f64 {
    record
        .metadata()
        .timestamp
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or_default()
}

/// Assert the JSON `created` field carries the record's timestamp as a number.
///
/// The comparison allows a one-ULP difference because the timestamp is
/// converted to `f64` once for the record and once here.
pub(super) fn assert_created_matches(parsed: &Value, record: &FemtoLogRecord) {
    let actual = parsed["created"]
        .as_f64()
        .expect("created should be a JSON number");
    let expected = created_seconds(record);
    assert!(
        (actual - expected).abs() <= f64::EPSILON * expected.abs(),
        "created {actual} should match the record timestamp {expected}"
    );
}

#[rstest]
pub(super) fn url_encoded_contains_expected_fields(test_record: FemtoLogRecord) {
    let encoded = serialize_url_encoded(&test_record, None).expect("serialize");
    assert_pairs_contain(
        &url_pairs(&encoded),
        &[
            ("name", "test.logger"),
            ("levelname", "INFO"),
            ("msg", "Hello+World"),
            ("lineno", "42"),
        ],
    );
}

#[rstest]
pub(super) fn json_contains_expected_fields(test_record: FemtoLogRecord) {
    let parsed = parsed_json(&test_record, None);
    assert_json_fields(
        &parsed,
        &[
            ("name", json!("test.logger")),
            ("levelname", json!("INFO")),
            ("msg", json!("Hello World")),
            ("lineno", json!(42)),
        ],
    );
}

#[rstest]
pub(super) fn field_filter_limits_output(test_record: FemtoLogRecord) {
    let fields = selected_fields(&["name", "msg"]);
    let json = serialize_json(&test_record, Some(&fields)).expect("serialize");
    let parsed: Value = serde_json::from_str(&json).expect("parse");

    assert_json_fields(
        &parsed,
        &[
            ("name", json!("test.logger")),
            ("msg", json!("Hello World")),
        ],
    );
    assert_json_keys_absent(&json, &["levelname", "lineno"]);
}
