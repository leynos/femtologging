//! Tests for ExceptionPayload recursive frame filtering methods.

use crate::exception_schema::*;
use crate::test_utils::frame_test_helpers::{
    assert_payload_frames, assert_payload_frames_by_function,
};
use rstest::rstest;

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
