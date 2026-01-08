//! Graceful degradation tests for traceback capture utilities.
//!
//! These tests verify that exception capture handles edge cases gracefully,
//! such as missing attributes, empty args, chained exceptions, and malformed notes.

use pyo3::prelude::*;
use pyo3::types::PyDict;
use rstest::rstest;

use crate::traceback_capture::capture_exception;

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
