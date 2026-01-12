//! Shared test utilities for exception schema and frame filtering tests.
//!
//! This module provides common assertion helpers and factory functions
//! for use across test modules.

use crate::exception_schema::{ExceptionPayload, StackFrame};

/// Create a StackFrame with the given filename, line number, and function name.
pub fn make_frame(filename: &str, lineno: u32, function: &str) -> StackFrame {
    StackFrame::new(filename, lineno, function)
}

/// Generic helper to assert a slice of frames has expected length and field values.
///
/// Validates that `expected_len` matches `expected_values.len()` to catch test
/// authoring errors.
pub fn assert_frames_by_field<F>(
    frames: &[StackFrame],
    expected_len: usize,
    expected_values: &[&str],
    field_extractor: F,
) where
    F: Fn(&StackFrame) -> &str,
{
    assert_eq!(
        expected_len,
        expected_values.len(),
        "expected_len ({}) must match expected_values.len() ({})",
        expected_len,
        expected_values.len()
    );
    assert_eq!(frames.len(), expected_len);
    for (i, expected) in expected_values.iter().enumerate() {
        assert_eq!(
            field_extractor(&frames[i]),
            *expected,
            "Mismatch at index {}",
            i
        );
    }
}

/// Assert frames have expected length and filenames.
pub fn assert_frames(frames: &[StackFrame], expected_len: usize, expected_filenames: &[&str]) {
    assert_frames_by_field(frames, expected_len, expected_filenames, |f| &f.filename);
}

/// Assert frames have expected length and function names.
pub fn assert_frames_by_function(
    frames: &[StackFrame],
    expected_len: usize,
    expected_functions: &[&str],
) {
    assert_frames_by_field(frames, expected_len, expected_functions, |f| &f.function);
}

/// Assert a payload's frames have expected length and filenames.
pub fn assert_payload_frames(
    payload: &ExceptionPayload,
    expected_len: usize,
    expected_filenames: &[&str],
) {
    assert_frames(&payload.frames, expected_len, expected_filenames);
}

/// Assert a payload's frames have expected length and function names.
pub fn assert_payload_frames_by_function(
    payload: &ExceptionPayload,
    expected_len: usize,
    expected_functions: &[&str],
) {
    assert_frames_by_function(&payload.frames, expected_len, expected_functions);
}
