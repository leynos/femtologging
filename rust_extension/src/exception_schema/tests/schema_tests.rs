//! Tests for exception schema type definitions and serialization.

use crate::exception_schema::*;
use rmp_serde::Serializer;
use rstest::rstest;
use serde::Serialize;
use std::collections::BTreeMap;

#[rstest]
fn schema_version_is_one() {
    assert_eq!(EXCEPTION_SCHEMA_VERSION, 1);
}

#[rstest]
fn stack_frame_new_sets_required_fields() {
    let frame = StackFrame::new("test.py", 42, "test_func");
    assert_eq!(frame.filename, "test.py");
    assert_eq!(frame.lineno, 42);
    assert_eq!(frame.function, "test_func");
    assert!(frame.end_lineno.is_none());
    assert!(frame.source_line.is_none());
    assert!(frame.locals.is_none());
}

#[rstest]
fn stack_frame_json_round_trip() {
    let mut locals = BTreeMap::new();
    locals.insert("x".into(), "42".into());

    let frame = StackFrame {
        filename: "example.py".into(),
        lineno: 10,
        end_lineno: Some(12),
        colno: Some(4),
        end_colno: Some(20),
        function: "process".into(),
        source_line: Some("    result = compute(x)".into()),
        locals: Some(locals),
    };

    let json = serde_json::to_string(&frame).expect("serialize frame");
    let decoded: StackFrame = serde_json::from_str(&json).expect("deserialize frame");
    assert_eq!(frame, decoded);
}

#[rstest]
fn stack_frame_skips_none_fields_in_json() {
    let frame = StackFrame::new("test.py", 1, "main");
    let json = serde_json::to_string(&frame).expect("serialize frame");
    assert!(!json.contains("end_lineno"));
    assert!(!json.contains("source_line"));
    assert!(!json.contains("locals"));
}

#[rstest]
fn stack_trace_payload_new_sets_version() {
    let payload = StackTracePayload::new(vec![]);
    assert_eq!(payload.schema_version, EXCEPTION_SCHEMA_VERSION);
}

#[rstest]
fn stack_trace_payload_json_round_trip() {
    let frames = vec![
        StackFrame::new("a.py", 1, "outer"),
        StackFrame::new("b.py", 2, "inner"),
    ];
    let payload = StackTracePayload::new(frames);

    let json = serde_json::to_string(&payload).expect("serialize");
    let decoded: StackTracePayload = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(payload, decoded);
}

#[rstest]
fn exception_payload_new_sets_version_and_message() {
    let payload = ExceptionPayload::new("KeyError", "missing key");
    assert_eq!(payload.schema_version, EXCEPTION_SCHEMA_VERSION);
    assert_eq!(payload.type_name, "KeyError");
    assert_eq!(payload.message, "missing key");
    assert!(payload.cause.is_none());
    assert!(payload.context.is_none());
}

#[rstest]
fn exception_payload_with_cause_chains() {
    let root = ExceptionPayload::new("IOError", "read failed");
    let wrapped = ExceptionPayload::new("RuntimeError", "operation failed").with_cause(root);

    assert!(wrapped.cause.is_some());
    let cause = wrapped.cause.as_ref().expect("cause exists");
    assert_eq!(cause.type_name, "IOError");
}

#[rstest]
fn exception_payload_with_context() {
    let ctx = ExceptionPayload::new("ValueError", "bad input");
    let error = ExceptionPayload::new("TypeError", "wrong type").with_context(ctx);

    assert!(error.context.is_some());
    assert!(!error.suppress_context);
}

#[rstest]
fn exception_payload_json_round_trip() {
    let frame = StackFrame::new("main.py", 100, "run");
    let cause = ExceptionPayload::new("OSError", "file not found");
    let payload = ExceptionPayload {
        schema_version: EXCEPTION_SCHEMA_VERSION,
        type_name: "RuntimeError".into(),
        module: Some("myapp.errors".into()),
        message: "failed to process".into(),
        args_repr: vec!["'path'".into(), "42".into()],
        notes: vec!["Check file permissions".into()],
        frames: vec![frame],
        cause: Some(Box::new(cause)),
        context: None,
        suppress_context: true,
        exceptions: vec![],
    };

    let json = serde_json::to_string(&payload).expect("serialize");
    let decoded: ExceptionPayload = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(payload, decoded);
}

#[rstest]
fn exception_payload_skips_default_fields() {
    let payload = ExceptionPayload::new("Error", "msg");
    let json = serde_json::to_string(&payload).expect("serialize");
    assert!(!json.contains("args_repr"));
    assert!(!json.contains("notes"));
    assert!(!json.contains("frames"));
    assert!(!json.contains("exceptions"));
    assert!(!json.contains("suppress_context"));
}

#[rstest]
fn exception_payload_includes_suppress_context_when_true() {
    let payload = ExceptionPayload {
        suppress_context: true,
        ..ExceptionPayload::new("Error", "msg")
    };
    let json = serde_json::to_string(&payload).expect("serialize");
    assert!(json.contains("suppress_context"));
}

#[rstest]
fn exception_payload_msgpack_round_trip() {
    let payload = ExceptionPayload::new("ValueError", "test")
        .with_frames(vec![StackFrame::new("test.py", 1, "main")]);

    // Use with_struct_map() for compatibility with deserialization
    let mut buf = Vec::new();
    payload
        .serialize(&mut Serializer::new(&mut buf).with_struct_map())
        .expect("serialize msgpack");
    let decoded: ExceptionPayload = rmp_serde::from_slice(&buf).expect("deserialize msgpack");
    assert_eq!(payload, decoded);
}

#[rstest]
fn exception_group_with_nested_exceptions() {
    let exc1 = ExceptionPayload::new("ValueError", "bad value 1");
    let exc2 = ExceptionPayload::new("TypeError", "wrong type");

    let group = ExceptionPayload {
        schema_version: EXCEPTION_SCHEMA_VERSION,
        type_name: "ExceptionGroup".into(),
        message: "multiple errors".into(),
        exceptions: vec![exc1, exc2],
        ..Default::default()
    };

    let json = serde_json::to_string(&group).expect("serialize");
    let decoded: ExceptionPayload = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded.exceptions.len(), 2);
}

/// Builds an exception chain of the specified type and depth.
///
/// # Arguments
/// * `chain_type` - "cause", "context", or "mixed" (alternates cause/context)
/// * `depth` - Total number of exceptions in the chain
fn build_exception_chain(chain_type: &str, depth: usize) -> ExceptionPayload {
    let mut current = ExceptionPayload::new("BaseError", "root");
    for i in 1..depth {
        current = match chain_type {
            "cause" => {
                ExceptionPayload::new(format!("Error{i}"), format!("level {i}")).with_cause(current)
            }
            "context" => ExceptionPayload::new(format!("Error{i}"), format!("context level {i}"))
                .with_context(current),
            "mixed" => {
                if i % 2 == 0 {
                    ExceptionPayload::new(format!("CauseError{i}"), format!("cause {i}"))
                        .with_cause(current)
                } else {
                    ExceptionPayload::new(format!("ContextError{i}"), format!("context {i}"))
                        .with_context(current)
                }
            }
            _ => panic!("Unknown chain type: {chain_type}"),
        };
    }
    current
}

/// Verifies that a payload has the expected chain depth by traversing links.
///
/// For "cause" chains, follows only cause links.
/// For "context" chains, follows only context links.
/// For "mixed" chains, follows whichever link exists at each level.
fn verify_chain_depth(payload: &ExceptionPayload, chain_type: &str, expected_depth: usize) {
    let mut depth = 0;
    let mut node = Some(payload);
    while let Some(n) = node {
        depth += 1;
        node = match chain_type {
            "cause" => n.cause.as_deref(),
            "context" => n.context.as_deref(),
            "mixed" => n.cause.as_deref().or(n.context.as_deref()),
            _ => panic!("Unknown chain type: {chain_type}"),
        };
    }
    assert_eq!(
        depth, expected_depth,
        "Expected chain depth {expected_depth}, found {depth}"
    );
}

#[rstest]
#[case("cause", 10)]
#[case("cause", 100)]
#[case("context", 100)]
#[case("mixed", 50)]
fn deep_chain_serializes(#[case] chain_type: &str, #[case] depth: usize) {
    let payload = build_exception_chain(chain_type, depth);

    let json = serde_json::to_string(&payload).expect("serialize deep chain");
    let decoded: ExceptionPayload = serde_json::from_str(&json).expect("deserialize");

    verify_chain_depth(&decoded, chain_type, depth);
}

/// Regression guard: 100-level cause chain should serialize in linear time.
///
/// This test uses a generous 10-second threshold to avoid flakiness in CI
/// while still catching quadratic or exponential time regressions.
#[rstest]
#[ignore = "timing-sensitive; run manually or via heavy-tests workflow"]
fn deep_cause_chain_100_levels_timing() {
    let start = std::time::Instant::now();

    let payload = build_exception_chain("cause", 100);

    let json = serde_json::to_string(&payload).expect("serialize deep chain");
    let _decoded: ExceptionPayload = serde_json::from_str(&json).expect("deserialize");

    let elapsed = start.elapsed();
    assert!(
        elapsed.as_secs() < 10,
        "Deep chain serialization took too long: {:?} (expected < 10s)",
        elapsed
    );
}

#[rstest]
fn types_are_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<StackFrame>();
    assert_send_sync::<StackTracePayload>();
    assert_send_sync::<ExceptionPayload>();
}

// Schema version validation tests

#[rstest]
#[case(1, true)]
#[case(EXCEPTION_SCHEMA_VERSION, true)]
#[case(0, false)]
#[case(EXCEPTION_SCHEMA_VERSION + 1, false)]
#[case(u16::MAX, false)]
fn validate_schema_version_cases(#[case] version: u16, #[case] valid: bool) {
    let result = validate_schema_version(version);
    assert_eq!(result.is_ok(), valid);
}

#[rstest]
#[case(EXCEPTION_SCHEMA_VERSION + 1, "VersionTooNew")]
#[case(0, "VersionTooOld")]
fn version_validation_error_includes_versions(
    #[case] version: u16,
    #[case] expected_variant: &str,
) {
    let err = validate_schema_version(version).expect_err("should fail for invalid version");

    match (&err, expected_variant) {
        (
            SchemaVersionError::VersionTooNew {
                found,
                max_supported,
            },
            "VersionTooNew",
        ) => {
            assert_eq!(*found, version);
            assert_eq!(*max_supported, EXCEPTION_SCHEMA_VERSION);
            let msg = err.to_string();
            assert!(msg.contains("maximum supported"), "should mention maximum");
            assert!(
                msg.contains(&version.to_string()),
                "error message should contain found version"
            );
        }
        (
            SchemaVersionError::VersionTooOld {
                found,
                min_supported,
            },
            "VersionTooOld",
        ) => {
            assert_eq!(*found, version);
            assert_eq!(*min_supported, MIN_EXCEPTION_SCHEMA_VERSION);
            let msg = err.to_string();
            assert!(msg.contains("minimum supported"), "should mention minimum");
            assert!(
                msg.contains(&version.to_string()),
                "error message should contain found version"
            );
        }
        _ => panic!("expected {expected_variant}, got {:?}", err),
    }
}

#[rstest]
fn exception_payload_validate_version_ok() {
    let payload = ExceptionPayload::new("ValueError", "test");
    assert!(payload.validate_version().is_ok());
}

#[rstest]
fn exception_payload_validate_version_future() {
    let mut payload = ExceptionPayload::new("ValueError", "test");
    payload.schema_version = EXCEPTION_SCHEMA_VERSION + 1;
    assert!(payload.validate_version().is_err());
}

#[rstest]
fn stack_trace_payload_validate_version_ok() {
    let payload = StackTracePayload::new(vec![]);
    assert!(payload.validate_version().is_ok());
}

#[rstest]
fn stack_trace_payload_validate_version_future() {
    let mut payload = StackTracePayload::new(vec![]);
    payload.schema_version = EXCEPTION_SCHEMA_VERSION + 1;
    assert!(payload.validate_version().is_err());
}

#[rstest]
fn deserialize_future_version_then_validate() {
    // Simulate receiving a payload with a higher schema version
    let json = r#"{
        "schema_version": 999,
        "type_name": "FutureError",
        "message": "from the future"
    }"#;

    // Deserialization succeeds (serde does not validate version)
    let payload: ExceptionPayload =
        serde_json::from_str(json).expect("deserialization should succeed");

    // Validation fails with informative error
    let err = payload
        .validate_version()
        .expect_err("validation should fail for future version");
    assert!(matches!(
        err,
        SchemaVersionError::VersionTooNew { found: 999, .. }
    ));
}

/// Minimal v1 payload JSON for backward compatibility tests.
fn minimal_v1_payload_json() -> &'static str {
    r#"{
        "schema_version": 1,
        "type_name": "Error",
        "message": "test"
    }"#
}

#[rstest]
fn backward_compatible_version_validation() {
    let payload: ExceptionPayload =
        serde_json::from_str(minimal_v1_payload_json()).expect("should deserialize");

    assert!(payload.validate_version().is_ok());
}

#[rstest]
fn backward_compatible_optional_field_defaults() {
    let payload: ExceptionPayload =
        serde_json::from_str(minimal_v1_payload_json()).expect("should deserialize");

    assert!(payload.module.is_none());
    assert!(payload.frames.is_empty());
    assert!(payload.cause.is_none());
}
