//! Regression tests for the `process_payload` example in the user guide.
//!
//! `build.rs` loads every marked Rust fence out of `docs/users-guide.md` into
//! `tests/generated`, and the `include!` below compiles the published text.
//! These tests therefore call the shipped example itself rather than a copy of
//! it, so the guide cannot drift from the behaviour it claims.

include!("generated/process_payload_schema_versioning.rs");

use rstest::rstest;

/// Minimal v1 payload that exercises the supported parsing path.
const MINIMAL_V1: &str = r#"{
    "schema_version": 1,
    "type_name": "Error",
    "message": "test"
}"#;

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

/// Assert that `process_payload` rejects a schema version as `expected`.
fn assert_version_error(schema_version: u16, expected: _femtologging_rs::SchemaVersionError) {
    let json = MINIMAL_V1.replace(
        "\"schema_version\": 1",
        &format!("\"schema_version\": {schema_version}"),
    );
    let err = process_payload(&json).expect_err("an unsupported version must fail");
    assert_eq!(
        err.downcast_ref::<_femtologging_rs::SchemaVersionError>(),
        Some(&expected)
    );
}

/// An unsupported schema version is reported after a successful parse.
#[rstest]
#[case(
    999,
    _femtologging_rs::SchemaVersionError::VersionTooNew {
        found: 999,
        max_supported: 1,
    }
)]
#[case(
    0,
    _femtologging_rs::SchemaVersionError::VersionTooOld {
        found: 0,
        min_supported: 1,
    }
)]
fn unsupported_version_is_reported(
    #[case] schema_version: u16,
    #[case] expected: _femtologging_rs::SchemaVersionError,
) {
    assert_version_error(schema_version, expected);
}
