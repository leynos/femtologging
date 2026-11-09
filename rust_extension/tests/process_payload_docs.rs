//! Regression tests for the `process_payload` example in the user guide.
//!
//! The guide publishes the example under a `tested-example` marker, but this
//! repository has no shared Markdown loader that compiles fenced Rust. The
//! function below mirrors the published text, and `guide_publishes_the_tested_function`
//! checks that the two stay in step.

use _femtologging_rs::{ExceptionPayload, SchemaVersionError, SchemaVersioned};

/// Marker that identifies the published example in `docs/users-guide.md`.
const MARKER: &str = "<!-- tested-example: process-payload-schema-versioning -->";

/// Minimal v1 payload that exercises the supported parsing path.
const MINIMAL_V1: &str = r#"{
    "schema_version": 1,
    "type_name": "Error",
    "message": "test"
}"#;

/// The example published in the user guide.
fn process_payload(json: &str) -> Result<(), Box<dyn std::error::Error>> {
    let payload: ExceptionPayload = serde_json::from_str(json)?;
    match payload.validate_version() {
        Ok(()) => Ok(()),
        Err(SchemaVersionError::VersionTooNew {
            found,
            max_supported,
        }) => Err(SchemaVersionError::VersionTooNew {
            found,
            max_supported,
        }
        .into()),
        Err(SchemaVersionError::VersionTooOld {
            found,
            min_supported,
        }) => Err(SchemaVersionError::VersionTooOld {
            found,
            min_supported,
        }
        .into()),
    }
}

/// Return the body of the Rust fence that follows the tested-example marker.
fn published_example() -> &'static str {
    let guide = include_str!("../../docs/users-guide.md");
    let after_marker = guide
        .split_once(MARKER)
        .expect("the guide should mark the process_payload example")
        .1;
    let after_newline = after_marker.strip_prefix('\n').unwrap_or(after_marker);
    let fence = after_newline
        .strip_prefix("```rust\n")
        .expect("the marker should precede a Rust fence");
    fence
        .split_once("\n```")
        .expect("the Rust fence should be terminated")
        .0
}

/// The guide publishes a fallible parser rather than a panicking one.
#[test]
fn guide_publishes_the_tested_function() {
    let example = published_example();
    assert!(example.contains("use _femtologging_rs::{"));
    assert!(example.contains("serde_json::from_str(json)?"));
    assert!(example.contains("match payload.validate_version()"));
    assert!(
        !example.contains("expect(\"valid JSON\")"),
        "the example must not panic on malformed JSON"
    );
}

/// Malformed JSON is propagated as an error, not a panic.
#[test]
fn malformed_json_returns_error_without_panicking() {
    let err = process_payload("{").expect_err("malformed JSON must fail");
    assert!(
        err.downcast_ref::<serde_json::Error>().is_some(),
        "expected serde_json::Error, got {err}"
    );
}

/// A supported payload parses and validates.
#[test]
fn valid_v1_payload_succeeds() {
    assert!(process_payload(MINIMAL_V1).is_ok());
}

/// A newer schema version is reported after a successful parse.
#[test]
fn too_new_version_is_reported() {
    let json = MINIMAL_V1.replace("\"schema_version\": 1", "\"schema_version\": 999");
    let err = process_payload(&json).expect_err("too-new version must fail");
    assert_eq!(
        err.downcast_ref::<SchemaVersionError>(),
        Some(&SchemaVersionError::VersionTooNew {
            found: 999,
            max_supported: 1,
        })
    );
}

/// An older schema version is reported after a successful parse.
#[test]
fn too_old_version_is_reported() {
    let json = MINIMAL_V1.replace("\"schema_version\": 1", "\"schema_version\": 0");
    let err = process_payload(&json).expect_err("too-old version must fail");
    assert_eq!(
        err.downcast_ref::<SchemaVersionError>(),
        Some(&SchemaVersionError::VersionTooOld {
            found: 0,
            min_supported: 1,
        })
    );
}
