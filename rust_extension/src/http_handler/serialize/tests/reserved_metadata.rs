//! Property tests for the reserved-metadata omission rule.
//!
//! The named cases in the parent module pin the hand-picked collisions that
//! matter most; these generated cases check the general rule over arbitrary
//! combinations of reserved and custom keys, which is where a serialization
//! regression is most likely to hide a single name from the shared contract.
//!
//! The expected classification comes from [`RESERVED_METADATA_KEYS`], not from
//! the production predicate, so a predicate that starts rejecting a near-miss
//! name such as `logger_name` fails these tests instead of silently shrinking
//! what they check.

use super::*;

use proptest::prelude::*;
use serde_json::{Value, json};

/// Every reserved name, mirroring the shared contract for generated cases.
const RESERVED_METADATA_KEYS: &[&str] = &[
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

/// Names that resemble reserved fields but are legitimate custom fields.
const NEAR_MISS_METADATA_KEYS: &[&str] = &["logger_name", "Name", "msg_id"];

/// Report whether the contract reserves `key`, judged only by this module.
fn is_contract_reserved(key: &str) -> bool {
    RESERVED_METADATA_KEYS.contains(&key)
}

/// Build the metadata pairs for one generated serialization case.
///
/// Reserved entries carry an unmistakable `SPOOF` marker so any leak is
/// detectable, while generated custom entries are filtered to keys the
/// contract treats as custom fields.
fn generated_metadata(
    reserved: Vec<(&'static str, String)>,
    custom: Vec<(String, String)>,
) -> Vec<(String, String)> {
    let mut key_values: Vec<(String, String)> = reserved
        .into_iter()
        .map(|(key, value)| (key.to_owned(), format!("SPOOF{value}")))
        .collect();
    key_values.extend(
        custom
            .into_iter()
            .filter(|(key, _)| !is_contract_reserved(key)),
    );
    // Metadata is a map, so a repeated generated key keeps one value.
    key_values.sort();
    key_values.dedup_by(|left, right| left.0 == right.0);
    key_values
}

/// Assert neither wire format leaks a reserved marker value at top level.
fn assert_no_spoofed_value_reaches_the_wire(json: &str, encoded: &str) {
    let parsed: Value = serde_json::from_str(json).expect("parse");
    for (key, value) in parsed.as_object().expect("JSON object") {
        if let Some(text) = value.as_str() {
            assert!(
                !text.starts_with("SPOOF"),
                "reserved metadata reached JSON as '{key}'"
            );
        }
    }
    for (key, value) in url_pairs(encoded) {
        assert!(
            !value.starts_with("SPOOF"),
            "reserved metadata reached URL-encoded output as '{key}'"
        );
    }
}

/// Near-miss names are custom fields, so both encodings must keep them.
#[test]
fn near_miss_metadata_names_survive_both_encodings() {
    let pairs: Vec<(String, String)> = NEAR_MISS_METADATA_KEYS
        .iter()
        .map(|key| ((*key).to_owned(), format!("value-{key}")))
        .collect();
    let borrowed: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    let record = record_with_key_values(&borrowed);

    let json = serialize_json(&record, None).expect("serialize");
    let parsed: Value = serde_json::from_str(&json).expect("parse");
    let encoded = serialize_url_encoded(&record, None).expect("serialize");
    let url = url_pairs(&encoded);

    for (key, value) in &pairs {
        assert!(
            !is_contract_reserved(key),
            "{key} should be a custom field, not a reserved name",
        );
        assert_eq!(
            parsed.get(key.as_str()).and_then(Value::as_str),
            Some(value.as_str()),
            "near-miss field '{key}' should survive JSON serialization",
        );
        assert!(
            url.contains(&(key.as_str(), value.as_str())),
            "near-miss field '{key}' should survive URL encoding",
        );
    }
}

proptest! {
    /// Reserved metadata never reaches the wire, and custom fields survive.
    #[test]
    fn reserved_metadata_never_reaches_the_wire(
        reserved in prop::collection::vec(
            (
                prop::sample::select(RESERVED_METADATA_KEYS.to_vec()),
                "[a-z0-9]{1,8}",
            ),
            1..6,
        ),
        custom in prop::collection::vec(("[a-z][a-z0-9_]{1,10}", "[0-9]{1,8}"), 0..4),
    ) {
        let key_values = generated_metadata(reserved, custom);
        let pairs: Vec<(&str, &str)> = key_values
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        let record = record_with_key_values(&pairs);

        let json = serialize_json(&record, None).expect("serialize");
        let encoded = serialize_url_encoded(&record, None).expect("serialize");
        assert_no_spoofed_value_reaches_the_wire(&json, &encoded);

        let parsed: Value = serde_json::from_str(&json).expect("parse");
        prop_assert_eq!(&parsed["name"], &json!("test.logger"));
        let url = url_pairs(&encoded);
        for (key, value) in key_values
            .iter()
            .filter(|(key, _)| !is_contract_reserved(key))
        {
            prop_assert_eq!(
                parsed.get(key.as_str()).and_then(Value::as_str),
                Some(value.as_str()),
                "custom field '{}' should survive JSON serialization",
                key
            );
            prop_assert!(
                url.contains(&(key.as_str(), value.as_str())),
                "custom field '{}' should survive URL encoding in {}",
                key,
                encoded
            );
        }
    }

    /// A generated field selection keeps only the selected custom field.
    #[test]
    fn generated_field_selection_keeps_only_selected_custom_fields(
        kept_value in "[0-9]{1,8}",
        dropped_value in "[0-9]{1,8}",
        reserved_value in "[a-z0-9]{1,8}",
    ) {
        // `args` is reserved and has no built-in wire field, so selecting it
        // must still produce nothing even when metadata supplies a value.
        let spoofed_args = format!("SPOOF{reserved_value}");
        let record = record_with_key_values(&[
            ("args", spoofed_args.as_str()),
            ("kept_field", kept_value.as_str()),
            ("dropped_field", dropped_value.as_str()),
        ]);
        let fields: Vec<String> =
            ["name", "args", "kept_field"].iter().map(|f| (*f).to_owned()).collect();

        let json = serialize_json(&record, Some(&fields)).expect("serialize");
        let encoded = serialize_url_encoded(&record, Some(&fields)).expect("serialize");
        let parsed: Value = serde_json::from_str(&json).expect("parse");
        let url = url_pairs(&encoded);

        prop_assert_eq!(
            parsed.get("kept_field").and_then(Value::as_str),
            Some(kept_value.as_str()),
            "the selected custom field should survive JSON serialization",
        );
        prop_assert_eq!(
            parsed.get("name").and_then(Value::as_str),
            Some("test.logger"),
            "the selected authoritative field should keep its built-in value",
        );
        prop_assert!(
            parsed.get("dropped_field").is_none(),
            "an unselected custom field must not reach JSON",
        );
        prop_assert!(
            parsed.get("args").is_none(),
            "selected reserved metadata must not reach JSON",
        );
        prop_assert!(
            url.contains(&("kept_field", kept_value.as_str())),
            "the selected custom field should survive URL encoding in {}",
            encoded,
        );
        prop_assert!(
            !url.iter().any(|(key, _)| *key == "dropped_field"),
            "an unselected custom field must not reach URL encoding",
        );
        prop_assert!(
            !url.iter().any(|(key, _)| *key == "args"),
            "selected reserved metadata must not reach URL encoding",
        );
        assert_no_spoofed_value_reaches_the_wire(&json, &encoded);
    }
}
