//! Property tests for the reserved-metadata omission rule.
//!
//! The named cases in the parent module pin the hand-picked collisions that
//! matter most; these generated cases check the general rule over arbitrary
//! combinations of reserved and custom keys, which is where a serialization
//! regression is most likely to hide a single name from the shared contract.

use super::*;

use crate::log_context::is_reserved_structured_key;
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

/// Build the metadata pairs for one generated serialization case.
///
/// Reserved entries carry an unmistakable `SPOOF` marker so any leak is
/// detectable, while generated custom entries are filtered to keys the shared
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
            .filter(|(key, _)| !is_reserved_structured_key(key)),
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
            .filter(|(key, _)| !is_reserved_structured_key(key))
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
}
