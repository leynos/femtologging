//! Graceful degradation tests for traceback capture utilities.
//!
//! These tests verify that exception capture handles edge cases gracefully,
//! such as missing attributes, empty args, chained exceptions, and malformed notes.

use pyo3::prelude::*;
use pyo3::types::PyDict;
use rstest::rstest;

use crate::traceback_capture::capture_exception;

// --------------------------------
// Helper functions for common setup
// --------------------------------

/// Create a ValueError instance with the given message.
fn create_value_error<'py>(py: Python<'py>, message: &str) -> Bound<'py, PyAny> {
    let builtins = py.import("builtins").expect("builtins module should exist");
    builtins
        .getattr("ValueError")
        .expect("ValueError should exist")
        .call1((message,))
        .expect("ValueError constructor should succeed")
}

/// Create a BaseException instance with no arguments.
fn create_base_exception<'py>(py: Python<'py>) -> Bound<'py, PyAny> {
    let builtins = py.import("builtins").expect("builtins module should exist");
    builtins
        .getattr("BaseException")
        .expect("BaseException should exist")
        .call0()
        .expect("BaseException constructor should succeed")
}

/// Check if Python supports add_note() via capability check (Python 3.11+).
///
/// Uses `hasattr(BaseException, "add_note")` to detect support, which is more
/// robust than version parsing and handles interpreter variants gracefully.
fn supports_add_note(py: Python<'_>) -> bool {
    let builtins = py.import("builtins").expect("builtins module should exist");
    let base_exception = builtins
        .getattr("BaseException")
        .expect("BaseException should exist");
    base_exception
        .hasattr("add_note")
        .expect("hasattr should succeed")
}

#[rstest]
fn capture_exception_without_notes_returns_empty_notes() {
    // Standard exceptions without __notes__ should return empty notes vector.
    // This tests the get_optional_attr degradation path for missing attributes.
    Python::with_gil(|py| {
        let exc = create_value_error(py, "test error");

        // Do not assert CPython internals/version-specific defaults for __notes__.
        // Only assert the capture output degrades to an empty notes vector.
        let payload = capture_exception(py, &exc)
            .expect("capture_exception should succeed")
            .expect("payload should be Some");

        assert!(
            payload.notes.is_empty(),
            "notes should be empty when __notes__ is missing/None/empty"
        );
        assert_eq!(payload.type_name, "ValueError");
    });
}

#[rstest]
fn capture_exception_with_empty_args_tuple() {
    // Exception with empty args tuple should produce empty args_repr.
    Python::with_gil(|py| {
        // BaseException() with no arguments has args = ()
        let exc = create_base_exception(py);

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
        // Skip test if Python version doesn't support add_note()
        if !supports_add_note(py) {
            // On Python < 3.11, run a simpler version without add_note()
            let code = c"
cause = ValueError('original error')

try:
    raise cause
except ValueError as e:
    raise RuntimeError('wrapped error') from e
";
            let result = py.run(code, None, None);
            assert!(result.is_err());

            let err = result.expect_err("code should raise an exception");
            let exc_value = err.value(py);

            let payload = capture_exception(py, exc_value)
                .expect("capture_exception should succeed")
                .expect("payload should be Some");

            assert_eq!(payload.type_name, "RuntimeError");
            assert!(
                !payload.args_repr.is_empty(),
                "outer exception should have args_repr"
            );

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
            return;
        }

        // Python 3.11+: test with add_note()
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
        let exc = create_value_error(py, "test error");

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
    // Per ADR "partial extraction of collections" rule: non-string entries are
    // dropped/ignored but valid strings are preserved.
    Python::with_gil(|py| {
        let exc = create_value_error(py, "test error");

        // Create a list with mixed types: valid string, integer, bytes, None
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

        assert_eq!(payload.type_name, "ValueError");

        // Per ADR "partial extraction of collections" rule: only valid string
        // entries are preserved; non-strings are skipped.
        assert_eq!(
            payload.notes,
            vec!["valid note"],
            "only valid string notes should survive; non-strings should be skipped"
        );
    });
}
