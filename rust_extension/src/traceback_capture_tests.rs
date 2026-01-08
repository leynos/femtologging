//! Unit tests for traceback capture utilities.

use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyTuple};
use rstest::rstest;

use crate::exception_schema::EXCEPTION_SCHEMA_VERSION;
use crate::traceback_capture::{capture_exception, capture_stack};

#[rstest]
fn capture_exception_with_true_no_active_exception() {
    Python::with_gil(|py| {
        let true_val = PyBool::new(py, true);
        let result = capture_exception(py, true_val.as_any())
            .expect("capture_exception should not fail with True");
        assert!(result.is_none(), "No active exception should return None");
    });
}

#[rstest]
fn capture_exception_with_false_returns_none() {
    Python::with_gil(|py| {
        let false_val = PyBool::new(py, false);
        let result = capture_exception(py, false_val.as_any())
            .expect("capture_exception should not fail with False");
        assert!(result.is_none());
    });
}

#[rstest]
fn capture_exception_with_instance() {
    Python::with_gil(|py| {
        // Create an exception instance
        let exc = py
            .import("builtins")
            .expect("builtins module should exist")
            .getattr("ValueError")
            .expect("ValueError should exist")
            .call1(("test error",))
            .expect("ValueError constructor should succeed");

        let result = capture_exception(py, &exc)
            .expect("capture_exception should succeed with exception instance");
        assert!(result.is_some());

        let payload = result.expect("payload should be Some for valid exception");
        assert_eq!(payload.type_name, "ValueError");
        assert_eq!(payload.message, "test error");
        assert_eq!(payload.schema_version, EXCEPTION_SCHEMA_VERSION);
    });
}

#[rstest]
fn capture_exception_with_tuple() {
    Python::with_gil(|py| {
        // Create a 3-tuple (type, value, traceback)
        let exc_type = py
            .import("builtins")
            .expect("builtins module should exist")
            .getattr("KeyError")
            .expect("KeyError should exist");
        let exc_value = exc_type
            .call1(("missing_key",))
            .expect("KeyError constructor should succeed");
        let exc_tb = py.None();

        let tuple = PyTuple::new(
            py,
            &[exc_type.as_any(), exc_value.as_any(), exc_tb.bind(py)],
        )
        .expect("tuple creation should succeed");

        let result = capture_exception(py, tuple.as_any())
            .expect("capture_exception should succeed with tuple");
        assert!(result.is_some());

        let payload = result.expect("payload should be Some for valid tuple");
        assert_eq!(payload.type_name, "KeyError");
    });
}

#[rstest]
fn capture_exception_with_none_value_tuple() {
    Python::with_gil(|py| {
        // 3-tuple with None value means no exception
        let none = py.None();
        let tuple = PyTuple::new(py, &[none.bind(py), none.bind(py), none.bind(py)])
            .expect("tuple creation should succeed");

        let result = capture_exception(py, tuple.as_any())
            .expect("capture_exception should not fail with None-value tuple");
        assert!(result.is_none());
    });
}

#[rstest]
fn capture_exception_tuple_preserves_explicit_traceback() {
    Python::with_gil(|py| {
        // Raise an exception to get a real traceback, then clear __traceback__
        // but pass the traceback explicitly in the tuple - frames should be preserved
        let code = c"
def inner():
    raise ValueError('test error')

def outer():
    inner()

try:
    outer()
except ValueError as e:
    exc_type = type(e)
    exc_value = e
    exc_tb = e.__traceback__
    # Clear the exception's __traceback__ to simulate the case where
    # it has been garbage collected or explicitly cleared
    e.__traceback__ = None
";
        let globals = PyDict::new(py);
        py.run(code, Some(&globals), None)
            .expect("code to raise and capture exception should succeed");

        let exc_type = globals
            .get_item("exc_type")
            .expect("get_item should not fail")
            .expect("exc_type should exist");
        let exc_value = globals
            .get_item("exc_value")
            .expect("get_item should not fail")
            .expect("exc_value should exist");
        let exc_tb = globals
            .get_item("exc_tb")
            .expect("get_item should not fail")
            .expect("exc_tb should exist");

        // Verify __traceback__ is None on the exception
        let current_tb = exc_value
            .getattr("__traceback__")
            .expect("should have __traceback__ attr");
        assert!(
            current_tb.is_none(),
            "exception's __traceback__ should be None"
        );

        // Create tuple with explicit traceback
        let tuple = PyTuple::new(py, &[&exc_type, &exc_value, &exc_tb])
            .expect("tuple creation should succeed");

        let result =
            capture_exception(py, tuple.as_any()).expect("capture_exception should succeed");
        let payload = result.expect("payload should be Some");

        assert_eq!(payload.type_name, "ValueError");
        assert_eq!(payload.message, "test error");

        // Frames should be preserved from the explicit traceback
        assert!(
            !payload.frames.is_empty(),
            "frames should be preserved from explicit traceback"
        );

        // Verify we have the expected call stack (inner, outer, <module>)
        let function_names: Vec<&str> =
            payload.frames.iter().map(|f| f.function.as_str()).collect();
        assert!(
            function_names.contains(&"inner"),
            "should contain 'inner' frame, got: {function_names:?}"
        );
        assert!(
            function_names.contains(&"outer"),
            "should contain 'outer' frame, got: {function_names:?}"
        );
    });
}

#[rstest]
fn capture_exception_invalid_type_raises_error() {
    Python::with_gil(|py| {
        let code = c"42";
        let invalid = py
            .eval(code, None, None)
            .expect("eval of integer literal should succeed");
        let result = capture_exception(py, &invalid);
        assert!(result.is_err());
    });
}

#[rstest]
fn capture_exception_with_chained_cause() {
    Python::with_gil(|py| {
        let code =
            c"try:\n    raise IOError('read failed')\nexcept IOError as e:\n    raise RuntimeError('operation failed') from e\n";

        // Execute code and capture the exception
        let result = py.run(code, None, None);
        assert!(result.is_err());

        let err = result.expect_err("code should raise an exception");
        let exc_value = err.value(py);

        let payload = capture_exception(py, exc_value)
            .expect("capture_exception should succeed")
            .expect("payload should be Some for chained exception");

        assert_eq!(payload.type_name, "RuntimeError");
        assert!(payload.cause.is_some());

        let cause = payload.cause.expect("cause should be Some");
        // IOError is an alias for OSError in Python 3
        assert_eq!(cause.type_name, "OSError");
        assert_eq!(cause.message, "read failed");
    });
}

#[rstest]
fn capture_stack_returns_frames() {
    Python::with_gil(|py| {
        let payload = capture_stack(py).expect("capture_stack should succeed");
        assert_eq!(payload.schema_version, EXCEPTION_SCHEMA_VERSION);
        assert!(!payload.frames.is_empty(), "Stack should have frames");

        // Check that frames have required fields
        let frame = &payload.frames[0];
        assert!(!frame.filename.is_empty());
        assert!(!frame.function.is_empty());
    });
}

#[rstest]
fn capture_exception_with_notes() {
    Python::with_gil(|py| {
        // Create an exception with notes (Python 3.11+)
        let code = c"e = ValueError('test'); e.add_note('Note 1'); e.add_note('Note 2')";
        let globals = PyDict::new(py);
        py.run(code, Some(&globals), None)
            .expect("code to create exception with notes should succeed");
        let exc = globals
            .get_item("e")
            .expect("get_item should not fail")
            .expect("exception 'e' should exist in globals");

        let payload = capture_exception(py, &exc)
            .expect("capture_exception should succeed")
            .expect("payload should be Some");

        assert_eq!(payload.notes.len(), 2);
        assert_eq!(payload.notes[0], "Note 1");
        assert_eq!(payload.notes[1], "Note 2");
    });
}

#[rstest]
fn capture_exception_args_repr() {
    Python::with_gil(|py| {
        let exc = py
            .import("builtins")
            .expect("builtins module should exist")
            .getattr("ValueError")
            .expect("ValueError should exist")
            .call1(("message", 42))
            .expect("ValueError constructor should succeed");

        let payload = capture_exception(py, &exc)
            .expect("capture_exception should succeed")
            .expect("payload should be Some");

        assert_eq!(payload.args_repr.len(), 2);
        assert_eq!(payload.args_repr[0], "'message'");
        assert_eq!(payload.args_repr[1], "42");
    });
}

#[rstest]
fn types_are_send_and_sync() {
    use crate::exception_schema::{ExceptionPayload, StackTracePayload};

    // Verify that capture functions can be used across threads
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    assert_send::<ExceptionPayload>();
    assert_sync::<ExceptionPayload>();
    assert_send::<StackTracePayload>();
    assert_sync::<StackTracePayload>();
}

// --------------------------------
// Graceful degradation tests
// --------------------------------

#[rstest]
fn capture_exception_without_notes_returns_empty_notes() {
    // Standard exceptions without __notes__ should return empty notes vector.
    // This tests the get_optional_attr degradation path for missing attributes.
    Python::with_gil(|py| {
        let exc = py
            .import("builtins")
            .expect("builtins module should exist")
            .getattr("ValueError")
            .expect("ValueError should exist")
            .call1(("test error",))
            .expect("ValueError constructor should succeed");

        // Verify the exception has no __notes__ attribute
        assert!(
            exc.getattr("__notes__").is_err()
                || exc
                    .getattr("__notes__")
                    .map(|n| n.is_none())
                    .unwrap_or(true),
            "fresh exception should have no __notes__"
        );

        let payload = capture_exception(py, &exc)
            .expect("capture_exception should succeed")
            .expect("payload should be Some");

        // notes should be empty (graceful degradation from missing attribute)
        assert!(
            payload.notes.is_empty(),
            "notes should be empty when __notes__ is missing"
        );
        assert_eq!(payload.type_name, "ValueError");
    });
}

#[rstest]
fn capture_exception_with_empty_args_tuple() {
    // Exception with empty args tuple should produce empty args_repr.
    Python::with_gil(|py| {
        // BaseException() with no arguments has args = ()
        let exc = py
            .import("builtins")
            .expect("builtins module should exist")
            .getattr("BaseException")
            .expect("BaseException should exist")
            .call0()
            .expect("BaseException constructor should succeed");

        let payload = capture_exception(py, &exc)
            .expect("capture_exception should succeed")
            .expect("payload should be Some");

        // args_repr should be empty because args tuple is empty
        assert!(
            payload.args_repr.is_empty(),
            "args_repr should be empty for exception with no arguments"
        );
        assert_eq!(payload.type_name, "BaseException");
    });
}

#[rstest]
fn capture_exception_chained_cause_has_empty_notes_and_args() {
    // Chained exceptions (where we only have TracebackException, not the instance)
    // should have empty notes and args_repr because those require the original instance.
    Python::with_gil(|py| {
        // Create a chained exception where the cause has notes
        let code = c"
cause = ValueError('original error')
cause.add_note('This note should NOT appear in the captured cause')

try:
    raise cause
except ValueError as e:
    raise RuntimeError('wrapped error') from e
";
        // Execute code and capture the exception
        let result = py.run(code, None, None);
        assert!(result.is_err());

        let err = result.expect_err("code should raise an exception");
        let exc_value = err.value(py);

        let payload = capture_exception(py, exc_value)
            .expect("capture_exception should succeed")
            .expect("payload should be Some");

        // The outer exception should have args_repr captured
        assert_eq!(payload.type_name, "RuntimeError");
        assert!(
            !payload.args_repr.is_empty(),
            "outer exception should have args_repr"
        );

        // The chained cause should exist but have empty notes and args_repr
        // because we don't have direct access to the exception instance
        let cause = payload.cause.expect("cause should be present");
        assert_eq!(cause.type_name, "ValueError");
        assert!(
            cause.notes.is_empty(),
            "chained exception notes should be empty (no instance access)"
        );
        assert!(
            cause.args_repr.is_empty(),
            "chained exception args_repr should be empty (no instance access)"
        );
    });
}

#[rstest]
fn capture_exception_with_non_iterable_notes_degrades_gracefully() {
    // Exceptions with a non-iterable __notes__ attribute should degrade gracefully.
    // This tests the type-mismatch degradation path for __notes__.
    Python::with_gil(|py| {
        let exc = py
            .import("builtins")
            .expect("builtins module should exist")
            .getattr("ValueError")
            .expect("ValueError should exist")
            .call1(("test error",))
            .expect("ValueError constructor should succeed");

        // Set a malformed, non-iterable __notes__ value (integer instead of list)
        exc.setattr("__notes__", 123_i32)
            .expect("setting non-iterable __notes__ should succeed at Python level");

        let payload = capture_exception(py, &exc)
            .expect("capture_exception should succeed even with malformed __notes__")
            .expect("payload should be Some");

        // For non-iterable __notes__, we expect graceful degradation to empty notes
        assert!(
            payload.notes.is_empty(),
            "expected empty notes for non-iterable __notes__, got: {:?}",
            payload.notes
        );
        assert_eq!(payload.type_name, "ValueError");
    });
}

#[rstest]
fn capture_exception_with_non_string_note_elements_degrades_gracefully() {
    // Exceptions with __notes__ containing non-string elements should still
    // be captured without failing, degrading notes content as needed.
    Python::with_gil(|py| {
        let exc = py
            .import("builtins")
            .expect("builtins module should exist")
            .getattr("ValueError")
            .expect("ValueError should exist")
            .call1(("test error",))
            .expect("ValueError constructor should succeed");

        // Create a list with mixed types: valid string, integer, bytes
        let code = c"
notes_list = ['valid note', 42, b'bytes note', None]
";
        let globals = PyDict::new(py);
        py.run(code, Some(&globals), None)
            .expect("code to create notes list should succeed");
        let notes_list = globals
            .get_item("notes_list")
            .expect("get_item should not fail")
            .expect("notes_list should exist");

        exc.setattr("__notes__", notes_list)
            .expect("setting list-valued __notes__ should succeed");

        let payload = capture_exception(py, &exc)
            .expect("capture_exception should succeed even with malformed note entries")
            .expect("payload should be Some");

        // The implementation should degrade gracefully - either skipping bad entries
        // or stringifying them. We verify it doesn't error and produces a result.
        assert_eq!(payload.type_name, "ValueError");

        // At minimum, the valid string note should be captured
        // (implementation may vary in handling of non-string elements)
        assert!(
            payload.notes.iter().any(|n| n.contains("valid note")),
            "valid string note should be captured, got: {:?}",
            payload.notes
        );
    });
}
