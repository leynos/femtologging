//! Shared arrangements for bounded-capture unit tests.
//!
//! The capture-limit tests and the graceful-degradation tests both need a real
//! Python exception to hand to capture, and both need a way to reach the
//! truncation marker. Keeping those arrangements here stops each test module
//! from growing its own copy and keeps both files inside the repository's line
//! limit.

use std::ffi::CStr;

use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::exception_schema::{ExceptionPayload, PayloadTruncation};
use crate::traceback_capture::{CaptureLimits, capture_exception_with_limits};

/// Limits that put every field at or near the smallest useful size.
///
/// The frame, node and aggregate ceilings stay below the defaults so that a
/// short test can reach both the per-field limits and the whole-capture budget.
pub fn tiny_limits() -> CaptureLimits {
    CaptureLimits {
        max_message_bytes: 32,
        max_args: 2,
        max_arg_bytes: 16,
        max_notes: 2,
        max_note_bytes: 16,
        max_frames: 3,
        max_source_line_bytes: 32,
        max_filename_bytes: 24,
        max_function_bytes: 24,
        max_chain_depth: 2,
        max_group_depth: 2,
        max_group_width: 2,
        max_nodes: 8,
        max_aggregate_bytes: 512,
    }
}

/// Run `source` in a fresh namespace and capture the exception it raised.
///
/// `source` must end by raising, which keeps every test's exception shape
/// visible in the test itself rather than in a shared helper.
pub fn capture_raised(py: Python<'_>, source: &CStr, limits: &CaptureLimits) -> ExceptionPayload {
    let globals = PyDict::new(py);
    capture_raised_in(py, source, limits, &globals)
}

/// Run `source` in `globals` and capture the exception it raised.
///
/// Taking the namespace from the caller lets a test read a binding the script
/// left behind — an argument's `__repr__` call counter, say — which a private
/// namespace would hide.
pub fn capture_raised_in(
    py: Python<'_>,
    source: &CStr,
    limits: &CaptureLimits,
    globals: &Bound<'_, PyDict>,
) -> ExceptionPayload {
    let error = py
        .run(source, Some(globals), None)
        .expect_err("source should raise");
    capture_exception_with_limits(py, error.value(py), limits)
        .expect("capture should not raise")
        .expect("payload should be present")
}

/// Return the truncation marker, requiring one to be present.
pub fn marker(payload: &ExceptionPayload) -> &PayloadTruncation {
    payload
        .truncation
        .as_ref()
        .expect("payload should carry a truncation marker")
}
