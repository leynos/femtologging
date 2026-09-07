//! Assertion helpers for [`StackFrame`] values and extracted locals maps.
//!
//! These are pure assertions over already-extracted Rust data, kept apart from
//! the Python arrangement helpers in
//! [`crate::test_utils::traceback_test_helpers`] so that each module stays
//! within the repository line limit.

use std::collections::BTreeMap;

use crate::exception_schema::StackFrame;

/// Assert that a locals map contains the expected key-value pair.
///
/// # Arguments
///
/// * `locals` - The locals map to check
/// * `key` - The key that should be present in the map
/// * `expected` - The expected value for the key
///
/// # Panics
///
/// Panics if `key` is not present in `locals` or if its value differs from `expected`.
#[track_caller]
pub fn assert_local_equals(locals: &BTreeMap<String, String>, key: &str, expected: &str) {
    assert_eq!(
        locals.get(key).map(String::as_str),
        Some(expected),
        "locals[{key:?}] should equal {expected:?}"
    );
}

/// Assert that a locals map does not contain the specified key.
///
/// # Arguments
///
/// * `locals` - The locals map to check
/// * `key` - The key that should be absent from the map
/// * `reason` - A description of why the key should be absent (used in the assertion message)
///
/// # Panics
///
/// Panics if `key` is present in `locals`.
#[track_caller]
pub fn assert_local_absent(locals: &BTreeMap<String, String>, key: &str, reason: &str) {
    assert!(locals.get(key).is_none(), "{}", reason);
}

/// Assert that a stack frame has the expected required fields.
///
/// Verifies that the frame's `filename`, `lineno`, and `function` fields match
/// the expected values. These are the three required fields for every stack frame
/// per the exception schema.
///
/// # Arguments
///
/// * `frame` - The stack frame to verify
/// * `filename` - Expected filename for the frame
/// * `lineno` - Expected line number (1-indexed)
/// * `function` - Expected function name
///
/// # Panics
///
/// Panics if any of the required fields do not match the expected values.
#[track_caller]
pub fn assert_frame_required_fields(
    frame: &StackFrame,
    filename: &str,
    lineno: u32,
    function: &str,
) {
    assert_eq!(frame.filename, filename, "filename should match");
    assert_eq!(frame.lineno, lineno, "lineno should match");
    assert_eq!(frame.function, function, "function should match");
}

/// Expected values for optional frame fields in test assertions.
///
/// This struct provides a convenient way to specify expected values for
/// optional [`StackFrame`] fields. Use [`Default::default()`] when all
/// optional fields should be `None`.
///
/// # Fields
///
/// * `end_lineno` - Expected end line number, or `None` if absent
/// * `colno` - Expected column offset, or `None` if absent
/// * `end_colno` - Expected end column offset, or `None` if absent
/// * `source_line` - Expected source line text, or `None` if absent
#[derive(Clone, Copy, Default)]
pub struct ExpectedOptionalFields<'a> {
    /// Expected end line number for the frame.
    pub end_lineno: Option<u32>,
    /// Expected column offset for the frame.
    pub colno: Option<u32>,
    /// Expected end column offset for the frame.
    pub end_colno: Option<u32>,
    /// Expected source line text for the frame.
    pub source_line: Option<&'a str>,
}

/// Helper to assert that a frame's optional fields match expected values.
///
/// Compares each optional field of the [`StackFrame`] against the expected
/// values provided in [`ExpectedOptionalFields`].
///
/// # Arguments
///
/// * `frame` - The stack frame to verify
/// * `expected` - The expected values for optional fields
///
/// # Panics
///
/// Panics if any optional field does not match the expected value.
#[track_caller]
pub fn assert_frame_optional_fields(frame: &StackFrame, expected: ExpectedOptionalFields<'_>) {
    assert_eq!(
        frame.end_lineno, expected.end_lineno,
        "end_lineno should match"
    );
    assert_eq!(frame.colno, expected.colno, "colno should match");
    assert_eq!(
        frame.end_colno, expected.end_colno,
        "end_colno should match"
    );
    assert_eq!(
        frame.source_line.as_deref(),
        expected.source_line,
        "source_line should match"
    );
}
