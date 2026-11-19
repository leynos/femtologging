//! Unit tests for bounded stack capture.
//!
//! Stack capture is separated from the exception tests in
//! [`super::traceback_capture_limits_tests`] because it needs a fixture those
//! tests do not: a real Python caller. A Rust `#[test]` thread has no Python
//! frames at all, so these tests install the capture entry point into a Python
//! script and let that script call it. Everything here therefore exercises the
//! same path production takes, and the frame ceilings are measured against
//! chains assembled entirely out of Python `def`s.

use pyo3::prelude::*;
use pyo3::types::PyDict;
use rstest::rstest;

use crate::traceback_capture::{CaptureLimits, capture_stack_with_limits};

/// Capture the current stack for a Python caller under `max_frames`.
///
/// `capture_stack` records the frames of the Python code that called into the
/// extension. A Rust test thread has no Python frames at all, so a test that
/// wants a stack to bound must call this from Python; that is also the only
/// shape production ever uses. The helper is registered in a scratch module the
/// test builds for itself, so it is compiled only into test builds and never
/// joins the extension's public surface.
///
/// The ceiling is a parameter rather than a constant so each test states the
/// depth it is comparing against. A caller's stack contains only *Python*
/// frames — the Rust frames between the caller and this function are invisible
/// to `sys._getframe` — so the chain length a test can rely on is exactly the
/// number of nested `def`s it wrote, and the ceiling has to be chosen with that
/// count in hand.
#[pyfunction]
fn capture_stack_with_test_limits(
    py: Python<'_>,
    max_frames: usize,
) -> PyResult<(Vec<String>, Option<u32>)> {
    let limits = CaptureLimits {
        max_frames,
        ..CaptureLimits::default()
    };
    let payload = capture_stack_with_limits(py, &limits)?;
    let functions = payload
        .frames
        .iter()
        .map(|frame| frame.function.clone())
        .collect();
    // Keep `None` distinct from a marker with a zero count: a complete capture
    // must not carry a marker at all, and a helper that flattened both to `0`
    // could not show the difference.
    let omitted = payload
        .truncation
        .as_ref()
        .map(|marker| marker.frames_omitted);
    Ok((functions, omitted))
}

/// Run `source` in a fresh namespace with the stack helper and `max_frames`
/// installed, then call the zero-argument `entry` and read back its result.
///
/// The helper is deliberately absent from the real extension module, so the test
/// builds a throwaway module to wrap it in and copies the result into the
/// namespace the test script runs in. Nothing about the production surface has
/// to change to make the stack assertions possible.
///
/// The entry point is reached through the namespace's item mapping rather than
/// as an attribute, because a `dict` has no attributes for the names it maps.
/// The script is run first and the entry called afterwards, so the `run` frame
/// is gone by then: the stack the helper sees is exactly the `def`s the script
/// chained together, with nothing of the test harness on it.
fn capture_stack_from(
    py: Python<'_>,
    source: &std::ffi::CStr,
    entry: &str,
    max_frames: usize,
) -> (Vec<String>, Option<u32>) {
    let globals = PyDict::new(py);
    let module = PyModule::new(py, "_capture_limit_test_support")
        .expect("scratch module should be creatable");
    let helper = wrap_pyfunction!(capture_stack_with_test_limits, &module)
        .and_then(|function| module.add_function(function))
        .and_then(|()| module.getattr("capture_stack_with_test_limits"))
        .expect("stack helper should install");
    globals
        .set_item("capture_stack_with_test_limits", helper)
        .expect("helper should be visible to the script");
    globals
        .set_item("max_frames", max_frames)
        .expect("the ceiling should be visible to the script");
    py.run(source, Some(&globals), None)
        .expect("stack helper script should run");
    globals
        .get_item(entry)
        .expect("entry point lookup should succeed")
        .expect("script should define the entry point")
        .call0()
        .and_then(|result| result.extract())
        .expect("stack capture should return function names and an omission count")
}

#[rstest]
fn stack_capture_keeps_the_innermost_frames() {
    Python::attach(|py| {
        // Five nested calls against a ceiling of three. The script's own module
        // frame has already returned by the time the entry point is called, and
        // the Rust frames in between are invisible to `sys._getframe`, so the
        // chain is exactly the five `def`s below.
        let (functions, omitted) = capture_stack_from(
            py,
            c"
def level_four():
    return level_three()

def level_three():
    return level_two()

def level_two():
    return level_one()

def level_one():
    return capstack()

def capstack():
    return capture_stack_with_test_limits(max_frames)
",
            "level_four",
            3,
        );

        assert_eq!(
            functions,
            vec![
                "level_two".to_string(),
                "level_one".to_string(),
                "capstack".to_string()
            ],
            "the innermost frames should survive, in oldest-first order"
        );
        assert_eq!(
            omitted,
            Some(1),
            "a cut stack reports that a tail was omitted; the remaining frames \
             are not walked to count them"
        );
    });
}

#[rstest]
fn stack_capture_respects_the_frame_ceiling() {
    Python::attach(|py| {
        let (functions, omitted) = capture_stack_from(
            py,
            c"
def four():
    return three()

def three():
    return two()

def two():
    return one()

def one():
    return capture_stack_with_test_limits(max_frames)
",
            "four",
            2,
        );

        assert_eq!(functions.len(), 2, "the ceiling should bound the payload");
        assert_eq!(functions, vec!["two".to_string(), "one".to_string()]);
        assert_eq!(omitted, Some(1));
    });
}

#[rstest]
fn stack_capture_reports_no_marker_when_nothing_is_dropped() {
    Python::attach(|py| {
        let (functions, omitted) = capture_stack_from(
            py,
            c"
def outer():
    return inner()

def inner():
    return capture_stack_with_test_limits(max_frames)
",
            "outer",
            8,
        );

        assert_eq!(
            functions,
            vec!["outer".to_string(), "inner".to_string()],
            "a stack shorter than the ceiling is captured whole"
        );
        assert_eq!(
            omitted, None,
            "a complete capture should carry no truncation marker at all"
        );
    });
}
