//! Tests for HTTP record serialization.

use super::*;
use crate::exception_schema::{ExceptionPayload, StackFrame, StackTracePayload};
use crate::level::FemtoLevel;
use crate::log_record::RecordMetadata;
use rstest::{fixture, rstest};
use serde_json::{Value, json};

mod reserved_metadata;
mod support;
use support::*;

/// Build the standard record used across the serialization tests.
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

/// Reserved metadata names, plus one legitimate custom field.
const COLLIDING_KEY_VALUES: &[(&str, &str)] = &[
    ("name", "spoofed.logger"),
    ("levelname", "CRITICAL"),
    ("msg", "spoofed message"),
    ("created", "spoofed-created"),
    ("filename", "spoofed.rs"),
    ("lineno", "999"),
    ("module", "spoofed.module"),
    ("thread", "spoofed-thread"),
    ("threadName", "spoofed-thread-name"),
    ("exc_info", "spoofed-exc"),
    ("stack_info", "spoofed-stack"),
    ("logger", "spoofed-logger"),
    ("level", "CRITICAL"),
    ("message", "spoofed-message"),
    ("correlation_id", "abc-123"),
];

/// Reserved collisions whose built-in field is absent from the default record.
const ABSENT_BUILTIN_COLLISIONS: &[&str] =
    &["exc_info", "stack_info", "logger", "level", "message"];

/// Authoritative wire fields that must appear exactly once despite collisions.
const AUTHORITATIVE_COLLISION_FIELDS: &[&str] = &[
    "name",
    "levelname",
    "msg",
    "created",
    "filename",
    "lineno",
    "module",
    "thread",
    "threadName",
    "correlation_id",
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

/// Build a record carrying real exception and stack payloads.
fn record_with_payloads(key_values: &[(&str, &str)]) -> FemtoLogRecord {
    let frames = vec![StackFrame::new("app.py", 7, "handler")];
    record_with_key_values(key_values)
        .with_exception(ExceptionPayload::new("ValueError", "bad input"))
        .with_stack(StackTracePayload::new(frames))
}

/// Assert JSON keeps authoritative fields and drops reserved metadata.
#[rstest]
fn json_omits_reserved_metadata_and_keeps_authoritative_fields() {
    let record = record_with_authoritative_thread(COLLIDING_KEY_VALUES);
    let expected_thread = format!("{:?}", record.metadata().thread_id);

    let json = serialize_json(&record, None).expect("serialize");
    let parsed: Value = serde_json::from_str(&json).expect("parse");

    assert_json_fields(
        &parsed,
        &[
            ("name", json!("test.logger")),
            ("levelname", json!("INFO")),
            ("msg", json!("Hello World")),
            ("filename", json!("test.rs")),
            ("lineno", json!(42)),
            ("module", json!("test.module")),
            ("thread", json!(expected_thread)),
            ("threadName", json!("authoritative-thread")),
            ("correlation_id", json!("abc-123")),
        ],
    );
    assert_created_matches(&parsed, &record);
    assert_json_keys_unique(&json, AUTHORITATIVE_COLLISION_FIELDS);
    assert_json_keys_absent(&json, ABSENT_BUILTIN_COLLISIONS);
}

/// Assert reserved metadata is dropped even when its built-in field is absent.
#[rstest]
fn json_omits_reserved_metadata_even_when_builtin_field_is_absent() {
    let mut record = record_with_key_values(&[
        ("threadName", "spoofed-thread"),
        ("exc_info", "spoofed-exc"),
        ("stack_info", "spoofed-stack"),
    ]);
    record.metadata_mut().thread_name = None;

    let json = serialize_json(&record, None).expect("serialize");

    assert_json_keys_absent(&json, &["threadName", "exc_info", "stack_info"]);
}

/// Assert filtered JSON reports authoritative values for reserved names.
#[rstest]
fn filtered_json_uses_authoritative_values_for_reserved_fields() {
    let record = record_with_authoritative_thread(COLLIDING_KEY_VALUES);
    let expected_thread = format!("{:?}", record.metadata().thread_id);
    let fields = selected_fields(&[
        "name",
        "levelname",
        "msg",
        "created",
        "thread",
        "module",
        "correlation_id",
    ]);

    let field_names: Vec<&str> = fields.iter().map(String::as_str).collect();
    let json = serialize_json(&record, Some(&fields)).expect("serialize");
    let parsed: Value = serde_json::from_str(&json).expect("parse");

    assert_json_fields(
        &parsed,
        &[
            ("name", json!("test.logger")),
            ("levelname", json!("INFO")),
            ("msg", json!("Hello World")),
            ("thread", json!(expected_thread)),
            ("module", json!("test.module")),
            ("correlation_id", json!("abc-123")),
        ],
    );
    assert_created_matches(&parsed, &record);
    assert_json_keys_unique(&json, &field_names);
}

/// Assert metadata cannot displace the record's exception or stack payloads.
#[rstest]
fn json_keeps_record_payloads_when_metadata_collides_with_them() {
    let record =
        record_with_payloads(&[("exc_info", "spoofed-exc"), ("stack_info", "spoofed-stack")]);

    let json = serialize_json(&record, None).expect("serialize");
    let parsed: Value = serde_json::from_str(&json).expect("parse");

    assert_eq!(
        parsed["exc_info"]["type_name"], "ValueError",
        "the record's exception payload should be the only exc_info source"
    );
    assert_eq!(
        parsed["stack_info"]["frames"][0]["filename"], "app.py",
        "the record's stack payload should be the only stack_info source"
    );
    assert_json_keys_unique(&json, &["exc_info", "stack_info"]);
    assert!(
        !json.contains("spoofed"),
        "metadata must not replace the record's exception or stack payload"
    );
}

/// Assert URL-encoded output drops reserved metadata and keeps keys unique.
#[rstest]
fn url_encoded_omits_reserved_metadata_and_keeps_unique_keys() {
    let record = record_with_authoritative_thread(COLLIDING_KEY_VALUES);
    let expected_thread = url_encode(&format!("{:?}", record.metadata().thread_id));

    let encoded = serialize_url_encoded(&record, None).expect("serialize");
    let pairs = url_pairs(&encoded);

    assert_pairs_contain(
        &pairs,
        &[
            ("name", "test.logger"),
            ("levelname", "INFO"),
            ("msg", "Hello+World"),
            ("filename", "test.rs"),
            ("lineno", "42"),
            ("module", "test.module"),
            ("thread", expected_thread.as_str()),
            ("threadName", "authoritative-thread"),
            ("correlation_id", "abc-123"),
        ],
    );
    for field in AUTHORITATIVE_COLLISION_FIELDS {
        assert_pair_key_unique(&pairs, field);
    }
    assert_pairs_exclude(&pairs, ABSENT_BUILTIN_COLLISIONS);
}

/// Assert filtered URL-encoded output reports authoritative values for reserved names.
#[rstest]
fn filtered_url_encoded_uses_authoritative_values_for_reserved_fields() {
    let record = record_with_authoritative_thread(COLLIDING_KEY_VALUES);
    let expected_thread = url_encode(&format!("{:?}", record.metadata().thread_id));
    let fields = selected_fields(&[
        "name",
        "levelname",
        "msg",
        "created",
        "thread",
        "module",
        "correlation_id",
    ]);

    let encoded = serialize_url_encoded(&record, Some(&fields)).expect("serialize");
    let pairs = url_pairs(&encoded);

    assert_pairs_contain(
        &pairs,
        &[
            ("name", "test.logger"),
            ("levelname", "INFO"),
            ("msg", "Hello+World"),
            ("thread", expected_thread.as_str()),
            ("module", "test.module"),
            ("correlation_id", "abc-123"),
        ],
    );
    for field in &fields {
        assert_pair_key_unique(&pairs, field);
    }
}

/// Assert URL-encoded metadata cannot displace the record's payloads.
#[rstest]
fn url_encoded_keeps_record_payloads_when_metadata_collides_with_them() {
    let record =
        record_with_payloads(&[("exc_info", "spoofed-exc"), ("stack_info", "spoofed-stack")]);

    let encoded = serialize_url_encoded(&record, None).expect("serialize");
    let pairs = url_pairs(&encoded);

    assert_pair_key_unique(&pairs, "exc_info");
    assert_pair_key_unique(&pairs, "stack_info");
    assert!(
        !encoded.contains("spoofed"),
        "metadata must not replace the record's exception or stack payload"
    );
}

/// Assert custom fields stay flat and string-valued in both encodings.
#[rstest]
fn legitimate_custom_fields_remain_flat_and_string_valued() {
    let record =
        record_with_authoritative_thread(&[("correlation_id", "abc-123"), ("request_id", "req-1")]);

    let parsed = parsed_json(&record, None);
    assert_json_fields(
        &parsed,
        &[
            ("correlation_id", json!("abc-123")),
            ("request_id", json!("req-1")),
        ],
    );

    let encoded = serialize_url_encoded(&record, None).expect("serialize");
    assert!(
        url_pairs(&encoded).ends_with(&[("correlation_id", "abc-123"), ("request_id", "req-1")]),
        "custom fields should stay flat and string-valued in URL-encoded output"
    );
}

/// The wire order is part of the format, so a split serialiser must not move
/// `threadName` behind the caller's key/value pairs.
#[rstest]
fn thread_name_precedes_caller_key_values(test_record: FemtoLogRecord) {
    let mut metadata = test_record.metadata().clone();
    metadata.thread_name = Some("worker-1".to_owned());
    metadata
        .key_values
        .insert("request_id".to_owned(), "abc".to_owned());
    let record = FemtoLogRecord::with_metadata(
        test_record.logger(),
        test_record.level(),
        test_record.message(),
        metadata,
    );

    let json = serialize_json(&record, None).expect("serialize");
    let thread_name_at = json.find("\"threadName\"").expect("threadName is emitted");
    let key_at = json
        .find("\"request_id\"")
        .expect("key/value pair is emitted");
    assert!(
        thread_name_at < key_at,
        "threadName must stay ahead of caller keys: {json}"
    );
}
