//! Regression tests for the `process_payload` example in the user guide.
//!
//! `build.rs` loads every marked Rust fence out of `docs/users-guide.md` into
//! `tests/generated`, and the `include!` below compiles the published text.
//! These tests therefore call the shipped example itself rather than a copy of
//! it, so the guide cannot drift from the behaviour it claims.

include!("generated/process_payload_schema_versioning.rs");

use _femtologging_rs::{
    EXCEPTION_SCHEMA_VERSION, MAX_EXCEPTION_PAYLOAD_DEPTH, MAX_EXCEPTION_PAYLOAD_NODES,
    MIN_EXCEPTION_SCHEMA_VERSION,
};
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
///
/// The bounds come from the crate rather than from literals, so a schema bump
/// moves them with the contract. `u16::MAX` is newer than any supported
/// version, and version 0 is older than the range, which starts at 1.
#[rstest]
#[case(
    u16::MAX,
    _femtologging_rs::SchemaVersionError::VersionTooNew {
        found: u16::MAX,
        max_supported: _femtologging_rs::EXCEPTION_SCHEMA_VERSION,
    }
)]
#[case(
    0,
    _femtologging_rs::SchemaVersionError::VersionTooOld {
        found: 0,
        min_supported: _femtologging_rs::MIN_EXCEPTION_SCHEMA_VERSION,
    }
)]
fn unsupported_version_is_reported(
    #[case] schema_version: u16,
    #[case] expected: _femtologging_rs::SchemaVersionError,
) {
    assert_version_error(schema_version, expected);
}

/// JSON for a supported root whose nested `cause` carries `version`.
fn root_with_cause(version: u16) -> String {
    format!(
        concat!(
            r#"{{"schema_version": 1, "type_name": "Root", "message": "root", "#,
            r#""cause": {{"schema_version": {version}, "type_name": "Cause", "message": "cause"}}}}"#,
        ),
        version = version,
    )
}

/// JSON for a supported root whose nested `context` carries `version`.
fn root_with_context(version: u16) -> String {
    format!(
        concat!(
            r#"{{"schema_version": 1, "type_name": "Root", "message": "root", "#,
            r#""context": {{"schema_version": {version}, "type_name": "Context", "message": "context"}}}}"#,
        ),
        version = version,
    )
}

/// JSON for a group whose second member carries `version`.
fn group_with_second_member(version: u16) -> String {
    format!(
        concat!(
            r#"{{"schema_version": 1, "type_name": "Group", "message": "group", "#,
            r#""exceptions": [{{"schema_version": 1, "type_name": "Handled", "message": "handled"}}, "#,
            r#"{{"schema_version": {version}, "type_name": "Nested", "message": "nested"}}]}}"#,
        ),
        version = version,
    )
}

/// The mismatch `validate_schema_version` reports for `version`.
fn expected_mismatch(version: u16) -> _femtologging_rs::SchemaVersionError {
    if version > EXCEPTION_SCHEMA_VERSION {
        _femtologging_rs::SchemaVersionError::VersionTooNew {
            found: version,
            max_supported: EXCEPTION_SCHEMA_VERSION,
        }
    } else {
        _femtologging_rs::SchemaVersionError::VersionTooOld {
            found: version,
            min_supported: MIN_EXCEPTION_SCHEMA_VERSION,
        }
    }
}

/// Assert that `result` reports `expected` at the nested `path`.
#[track_caller]
fn assert_nested(
    result: Result<(), Box<dyn std::error::Error>>,
    path: &str,
    expected: _femtologging_rs::SchemaVersionError,
) {
    let err = result.expect_err("an unsupported nested version must fail");
    let schema_error = err
        .downcast_ref::<_femtologging_rs::SchemaVersionError>()
        .expect("expected a SchemaVersionError");
    match schema_error {
        _femtologging_rs::SchemaVersionError::Nested {
            path: actual_path,
            source,
        } => {
            assert_eq!(actual_path, path);
            assert_eq!(**source, expected);
        }
        other => panic!("expected Nested at {path}, got {other:?}"),
    }
}

/// Unsupported versions on every nested edge kind name their path.
#[rstest]
#[case(0)]
#[case(EXCEPTION_SCHEMA_VERSION + 1)]
fn a_nested_cause_mismatch_is_reported(#[case] version: u16) {
    assert_nested(
        process_payload(&root_with_cause(version)),
        "cause",
        expected_mismatch(version),
    );
}

/// A nested context mismatch is reported with its path.
#[rstest]
#[case(0)]
#[case(EXCEPTION_SCHEMA_VERSION + 1)]
fn a_nested_context_mismatch_is_reported(#[case] version: u16) {
    assert_nested(
        process_payload(&root_with_context(version)),
        "context",
        expected_mismatch(version),
    );
}

/// An exception-group member mismatch is reported with its index.
#[rstest]
#[case(0)]
#[case(EXCEPTION_SCHEMA_VERSION + 1)]
fn a_group_member_mismatch_is_reported(#[case] version: u16) {
    assert_nested(
        process_payload(&group_with_second_member(version)),
        "exceptions[1]",
        expected_mismatch(version),
    );
}

/// A fully supported nested tree passes the example's validation.
#[test]
fn a_fully_supported_nested_tree_succeeds() {
    let json = concat!(
        r#"{"schema_version": 1, "type_name": "Root", "message": "root", "#,
        r#""cause": {"schema_version": 1, "type_name": "Cause", "message": "cause"}, "#,
        r#""context": {"schema_version": 1, "type_name": "Context", "message": "context"}, "#,
        r#""exceptions": [{"schema_version": 1, "type_name": "Member", "message": "member"}]}"#,
    );

    assert!(process_payload(json).is_ok());
}

/// JSON for a group whose member count exceeds the node budget.
fn over_wide_group_json() -> String {
    let member = r#"{"schema_version": 1, "type_name": "Member", "message": "member"}"#;
    let members = std::iter::repeat_n(member, MAX_EXCEPTION_PAYLOAD_NODES)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        concat!(
            r#"{{"schema_version": 1, "type_name": "Group", "message": "group", "#,
            r#""exceptions": [{members}]}}"#,
        ),
        members = members,
    )
}

/// A payload tree past the node budget is rejected after a successful parse.
#[test]
fn an_over_wide_tree_reports_the_node_limit() {
    let err =
        process_payload(&over_wide_group_json()).expect_err("the tree exceeds the node limit");

    assert_eq!(
        err.downcast_ref::<_femtologging_rs::SchemaVersionError>(),
        Some(&_femtologging_rs::SchemaVersionError::TreeTooLarge {
            max_nodes: MAX_EXCEPTION_PAYLOAD_NODES,
        })
    );
}

/// JSON nesting cannot reach the validation depth limit.
///
/// `serde_json`'s default recursion limit is far below
/// `MAX_EXCEPTION_PAYLOAD_DEPTH`, so a JSON document deep enough for
/// `TreeTooDeep` fails to parse before the example can validate it.
#[test]
fn json_nesting_cannot_reach_the_validation_depth_limit() {
    let mut json = String::new();
    for _ in 0..=MAX_EXCEPTION_PAYLOAD_DEPTH {
        json.push_str(
            r#"{"schema_version": 1, "type_name": "Wrapper", "message": "wrapper", "cause": "#,
        );
    }
    json.push_str(r#"{"schema_version": 1, "type_name": "Leaf", "message": "leaf"}"#);
    for _ in 0..=MAX_EXCEPTION_PAYLOAD_DEPTH {
        json.push('}');
    }

    let err = process_payload(&json).expect_err("serde_json should reject the nesting");

    assert!(
        err.downcast_ref::<serde_json::Error>().is_some(),
        "expected a parse error, got {err}"
    );
}
