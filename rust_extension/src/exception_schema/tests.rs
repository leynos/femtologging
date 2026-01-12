//! Tests for exception schema types and frame filtering methods.

use super::*;
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

#[rstest]
fn deep_cause_chain_serializes() {
    // Test a chain of 10 nested causes to ensure no stack overflow
    let mut current = ExceptionPayload::new("BaseError", "root cause");
    for i in 1..10 {
        current =
            ExceptionPayload::new(format!("Error{i}"), format!("level {i}")).with_cause(current);
    }

    let json = serde_json::to_string(&current).expect("serialize deep chain");
    let decoded: ExceptionPayload = serde_json::from_str(&json).expect("deserialize");

    // Verify chain depth
    let mut depth = 0;
    let mut node = Some(&decoded);
    while let Some(n) = node {
        depth += 1;
        node = n.cause.as_deref();
    }
    assert_eq!(depth, 10);
}

#[rstest]
fn types_are_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<StackFrame>();
    assert_send_sync::<StackTracePayload>();
    assert_send_sync::<ExceptionPayload>();
}

// Tests for ExceptionPayload recursive filtering methods

/// Assert that a payload contains expected number of frames with specific filenames.
fn assert_payload_frames(
    payload: &ExceptionPayload,
    expected_len: usize,
    expected_filenames: &[&str],
) {
    assert_eq!(
        expected_len,
        expected_filenames.len(),
        "expected_len ({}) must match expected_filenames.len() ({})",
        expected_len,
        expected_filenames.len()
    );
    assert_eq!(payload.frames.len(), expected_len);
    for (i, expected) in expected_filenames.iter().enumerate() {
        assert_eq!(
            payload.frames[i].filename, *expected,
            "Mismatch at frame index {}",
            i
        );
    }
}

/// Assert that a payload contains expected number of frames with specific function names.
fn assert_payload_frames_by_function(
    payload: &ExceptionPayload,
    expected_len: usize,
    expected_functions: &[&str],
) {
    assert_eq!(
        expected_len,
        expected_functions.len(),
        "expected_len ({}) must match expected_functions.len() ({})",
        expected_len,
        expected_functions.len()
    );
    assert_eq!(payload.frames.len(), expected_len);
    for (i, expected) in expected_functions.iter().enumerate() {
        assert_eq!(
            payload.frames[i].function, *expected,
            "Mismatch at frame index {}",
            i
        );
    }
}

#[rstest]
fn exception_payload_limit_frames_recursive_on_cause() {
    let cause_frames = vec![
        StackFrame::new("cause_a.py", 1, "a"),
        StackFrame::new("cause_b.py", 2, "b"),
        StackFrame::new("cause_c.py", 3, "c"),
    ];
    let cause = ExceptionPayload::new("CauseError", "cause msg").with_frames(cause_frames);

    let main_frames = vec![
        StackFrame::new("main_a.py", 10, "main_a"),
        StackFrame::new("main_b.py", 20, "main_b"),
    ];
    let payload = ExceptionPayload::new("MainError", "main msg")
        .with_frames(main_frames)
        .with_cause(cause);

    let limited = payload.limit_frames(2);

    // Main frames limited to 2
    assert_payload_frames(&limited, 2, &["main_a.py", "main_b.py"]);

    // Cause frames also limited to 2 (last 2)
    let limited_cause = limited.cause.as_ref().expect("cause exists");
    assert_payload_frames(limited_cause, 2, &["cause_b.py", "cause_c.py"]);
}

#[rstest]
fn exception_payload_limit_frames_recursive_on_context() {
    let context_frames = vec![
        StackFrame::new("ctx_a.py", 1, "a"),
        StackFrame::new("ctx_b.py", 2, "b"),
        StackFrame::new("ctx_c.py", 3, "c"),
    ];
    let context = ExceptionPayload::new("ContextError", "ctx msg").with_frames(context_frames);

    let main_frames = vec![StackFrame::new("main.py", 10, "main")];
    let payload = ExceptionPayload::new("MainError", "main msg")
        .with_frames(main_frames)
        .with_context(context);

    let limited = payload.limit_frames(1);

    // Context frames limited to 1 (last 1)
    let limited_context = limited.context.as_ref().expect("context exists");
    assert_eq!(limited_context.frames.len(), 1);
    assert_eq!(limited_context.frames[0].filename, "ctx_c.py");
}

#[rstest]
fn exception_payload_filter_frames_recursive_on_cause_chain() {
    let inner_cause = ExceptionPayload::new("InnerCause", "inner").with_frames(vec![
        StackFrame::new("inner.py", 1, "inner_func"),
        StackFrame::new("logging/__init__.py", 2, "log"),
    ]);
    let outer_cause = ExceptionPayload::new("OuterCause", "outer")
        .with_frames(vec![
            StackFrame::new("outer.py", 10, "outer_func"),
            StackFrame::new("femtologging/handler.py", 20, "emit"),
        ])
        .with_cause(inner_cause);
    let payload = ExceptionPayload::new("TopError", "top")
        .with_frames(vec![StackFrame::new("top.py", 100, "top_func")])
        .with_cause(outer_cause);

    // Filter out logging infrastructure
    let filtered = payload
        .filter_frames(|f| !f.filename.contains("logging") && !f.filename.contains("femtologging"));

    // Top frame unchanged
    assert_eq!(filtered.frames.len(), 1);
    assert_eq!(filtered.frames[0].filename, "top.py");

    // Outer cause filtered
    let outer = filtered.cause.as_ref().expect("outer cause exists");
    assert_eq!(outer.frames.len(), 1);
    assert_eq!(outer.frames[0].filename, "outer.py");

    // Inner cause filtered
    let inner = outer.cause.as_ref().expect("inner cause exists");
    assert_eq!(inner.frames.len(), 1);
    assert_eq!(inner.frames[0].filename, "inner.py");
}

#[rstest]
fn exception_payload_exclude_filenames_recursive_on_exception_group() {
    let exc1 = ExceptionPayload::new("Error1", "err1").with_frames(vec![
        StackFrame::new("app/module1.py", 1, "func1"),
        StackFrame::new(".venv/lib/foo.py", 2, "venv_func"),
    ]);
    let exc2 = ExceptionPayload::new("Error2", "err2").with_frames(vec![
        StackFrame::new("app/module2.py", 10, "func2"),
        StackFrame::new("site-packages/bar.py", 20, "pkg_func"),
    ]);
    let group = ExceptionPayload {
        schema_version: EXCEPTION_SCHEMA_VERSION,
        type_name: "ExceptionGroup".into(),
        message: "multiple errors".into(),
        exceptions: vec![exc1, exc2],
        frames: vec![StackFrame::new("app/main.py", 100, "main")],
        ..Default::default()
    };

    let filtered = group.exclude_filenames(&[".venv/", "site-packages/"]);

    // Group's own frames unchanged
    assert_payload_frames(&filtered, 1, &["app/main.py"]);

    // exc1's venv frame removed
    assert_payload_frames(&filtered.exceptions[0], 1, &["app/module1.py"]);

    // exc2's site-packages frame removed
    assert_payload_frames(&filtered.exceptions[1], 1, &["app/module2.py"]);
}

#[rstest]
fn exception_payload_exclude_functions_recursive_on_cause_and_context() {
    let cause = ExceptionPayload::new("CauseError", "cause").with_frames(vec![
        StackFrame::new("cause.py", 1, "public_func"),
        StackFrame::new("cause.py", 2, "_private_helper"),
    ]);
    let context = ExceptionPayload::new("ContextError", "ctx").with_frames(vec![
        StackFrame::new("ctx.py", 10, "visible_func"),
        StackFrame::new("ctx.py", 20, "__internal"),
    ]);
    let payload = ExceptionPayload::new("MainError", "main")
        .with_frames(vec![
            StackFrame::new("main.py", 100, "main"),
            StackFrame::new("main.py", 110, "_setup"),
        ])
        .with_cause(cause)
        .with_context(context);

    let filtered = payload.exclude_functions(&["_private", "__internal", "_setup"]);

    // Main frames: _setup removed
    assert_payload_frames_by_function(&filtered, 1, &["main"]);

    // Cause frames: _private_helper removed
    let cause_result = filtered.cause.as_ref().expect("cause exists");
    assert_payload_frames_by_function(cause_result, 1, &["public_func"]);

    // Context frames: __internal removed
    let context_result = filtered.context.as_ref().expect("context exists");
    assert_payload_frames_by_function(context_result, 1, &["visible_func"]);
}

#[rstest]
fn exception_payload_exclude_logging_infrastructure_recursive() {
    let cause = ExceptionPayload::new("CauseError", "cause").with_frames(vec![
        StackFrame::new("cause.py", 1, "cause_func"),
        StackFrame::new("femtologging/__init__.py", 2, "log"),
    ]);
    let payload = ExceptionPayload::new("MainError", "main")
        .with_frames(vec![
            StackFrame::new("main.py", 100, "main"),
            StackFrame::new("logging/__init__.py", 110, "_log"),
        ])
        .with_cause(cause);

    let filtered = payload.exclude_logging_infrastructure();

    // Main frames: logging removed
    assert_payload_frames(&filtered, 1, &["main.py"]);

    // Cause frames: femtologging removed
    let cause_result = filtered.cause.as_ref().expect("cause exists");
    assert_payload_frames(cause_result, 1, &["cause.py"]);
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
fn version_too_new_error_includes_versions() {
    let future_version = EXCEPTION_SCHEMA_VERSION + 1;
    let err = validate_schema_version(future_version).expect_err("should fail for future version");

    match err {
        SchemaVersionError::VersionTooNew {
            found,
            max_supported,
        } => {
            assert_eq!(found, future_version);
            assert_eq!(max_supported, EXCEPTION_SCHEMA_VERSION);
        }
        SchemaVersionError::VersionTooOld { .. } => {
            panic!("expected VersionTooNew, got VersionTooOld");
        }
    }

    let msg = err.to_string();
    assert!(msg.contains("maximum supported"), "should mention maximum");
    assert!(
        msg.contains(&future_version.to_string()),
        "error message should contain found version"
    );
}

#[rstest]
fn version_too_old_error_includes_versions() {
    let old_version = 0;
    let err = validate_schema_version(old_version).expect_err("should fail for old version");

    match err {
        SchemaVersionError::VersionTooOld {
            found,
            min_supported,
        } => {
            assert_eq!(found, old_version);
            assert_eq!(min_supported, MIN_EXCEPTION_SCHEMA_VERSION);
        }
        SchemaVersionError::VersionTooNew { .. } => {
            panic!("expected VersionTooOld, got VersionTooNew");
        }
    }

    let msg = err.to_string();
    assert!(msg.contains("minimum supported"), "should mention minimum");
    assert!(
        msg.contains(&old_version.to_string()),
        "error message should contain found version"
    );
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
