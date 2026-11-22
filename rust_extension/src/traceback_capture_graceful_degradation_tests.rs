//! Graceful degradation tests for traceback capture utilities.
//!
//! These tests verify that exception capture handles edge cases gracefully,
//! such as missing attributes, empty args, chained exceptions, and malformed notes.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};
use rstest::rstest;

use crate::exception_schema::ExceptionPayload;
use crate::test_utils::capture_limit_helpers::capture_raised;
use crate::test_utils::traceback_test_helpers::{create_base_exception, create_value_error};
use crate::traceback_capture::{CaptureLimits, capture_exception};

// --------------------------------
// Helper functions for common setup
// --------------------------------

/// Check if Python supports add_note() via capability check (Python 3.11+).
///
/// Uses `hasattr(BaseException, "add_note")` to detect support, which is more
/// robust than version parsing and handles interpreter variants gracefully.
fn supports_add_note(py: Python<'_>) -> PyResult<bool> {
    py.import("builtins")?
        .getattr("BaseException")?
        .hasattr("add_note")
}

/// Borrow the chained cause of a payload, if one was captured.
///
/// A pure query over already-captured data; the absence of a cause is reported
/// to the caller rather than panicking here.
fn chained_cause(payload: &ExceptionPayload) -> Option<&ExceptionPayload> {
    payload.cause.as_deref()
}

/// Assert that the outer exception of a chained payload looks as expected.
#[track_caller]
fn assert_outer_exception(payload: &ExceptionPayload, outer_type: &str) {
    assert_eq!(
        payload.type_name, outer_type,
        "outer exception type should match"
    );
    assert!(
        !payload.args_repr.is_empty(),
        "outer exception should have args_repr"
    );
}

/// Assert that a chained cause has the expected type and no descendant-only data.
///
/// The payload contract leaves notes and `args_repr` empty on descendants, even
/// though the walker has access to their original exception instances.
#[track_caller]
fn assert_chained_cause(cause: &ExceptionPayload, cause_type: &str) {
    assert_eq!(
        cause.type_name, cause_type,
        "chained exception type should match"
    );
    assert!(
        cause.notes.is_empty(),
        "chained exception notes should be empty on descendants"
    );
    assert!(
        cause.args_repr.is_empty(),
        "chained exception args_repr should be empty on descendants"
    );
}

/// Assert that a chained exception payload has the expected structure.
///
/// A macro rather than a helper function so the missing-cause panic expands
/// inside the calling test and reports that test's location.
///
/// # Arguments
///
/// * `$payload` - The exception payload to verify
/// * `$outer_type` - Expected type name of the outer exception
/// * `$cause_type` - Expected type name of the chained cause
macro_rules! assert_chained_exception_structure {
    ($payload:expr, $outer_type:expr, $cause_type:expr) => {{
        let payload: &ExceptionPayload = $payload;
        assert_outer_exception(payload, $outer_type);
        let cause = chained_cause(payload).expect("cause should be present");
        assert_chained_cause(cause, $cause_type);
    }};
}

#[rstest]
fn capture_exception_with_empty_args_tuple() {
    // Exception with empty args tuple should produce empty args_repr.
    Python::attach(|py| {
        // BaseException() with no arguments has args = ()
        let exc = create_base_exception(py).expect("exception should build");

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
    // Descendant payloads omit notes and args_repr by contract, despite access
    // to the original exception instances.
    Python::attach(|py| {
        // Skip test if Python version doesn't support add_note()
        if !supports_add_note(py).expect("capability check should succeed") {
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

            assert_chained_exception_structure!(&payload, "RuntimeError", "ValueError");
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

        assert_chained_exception_structure!(&payload, "RuntimeError", "ValueError");
    });
}

#[rstest]
fn capture_exception_with_failing_repr_in_args_degrades_gracefully() {
    // When an exception's args tuple contains objects whose __repr__ raises,
    // those elements should be skipped and only valid repr strings preserved.
    Python::attach(|py| {
        // Create a class with a __repr__ that raises
        let code = c"
class BadRepr:
    def __repr__(self):
        raise RuntimeError('repr failed')
";
        let globals = PyDict::new(py);
        py.run(code, Some(&globals), None)
            .expect("class definition should succeed");
        let bad_repr_cls = globals
            .get_item("BadRepr")
            .expect("get_item should not fail")
            .expect("BadRepr class should exist in globals");
        let bad_repr_instance = bad_repr_cls.call0().expect("BadRepr() should succeed");

        // Create an exception and manually set args to include a failing repr
        let exc = create_value_error(py, "test error").expect("exception should build");
        let args_tuple = pyo3::types::PyTuple::new(
            py,
            [
                "valid arg"
                    .into_pyobject(py)
                    .expect("string conversion should succeed")
                    .into_any(),
                bad_repr_instance,
                42i32
                    .into_pyobject(py)
                    .expect("int conversion should succeed")
                    .into_any(),
            ],
        )
        .expect("tuple creation should succeed");
        exc.setattr("args", args_tuple)
            .expect("setting args should succeed");

        let payload = capture_exception(py, &exc)
            .expect("capture_exception should succeed even with failing repr")
            .expect("payload should be Some");

        assert_eq!(payload.type_name, "ValueError");

        // Per ADR "partial extraction of collections" rule: elements whose
        // repr() fails are skipped; valid repr strings are preserved.
        assert_eq!(
            payload.args_repr,
            vec!["'valid arg'", "42"],
            "only valid repr strings should survive; failing repr elements should be skipped"
        );
        let marker = payload
            .truncation
            .as_ref()
            .expect("a failed conversion should be recorded");
        assert_eq!(marker.failed_elements, 1);
        assert_eq!(
            marker.args_omitted, 0,
            "a failed conversion is not also an argument dropped by the ceiling"
        );
    });
}

#[rstest]
fn a_failing_descendant_traceback_read_is_recorded() {
    Python::attach(|py| {
        let payload = capture_raised(
            py,
            c"
class BadCause(Exception):
    @property
    def __traceback__(self):
        raise RuntimeError('no traceback')

error = ValueError('outer')
error.__cause__ = BadCause('inner')
raise error
",
            &CaptureLimits::default(),
        );

        let cause = payload
            .cause
            .as_ref()
            .expect("the assigned cause should be retained");
        assert_eq!(cause.type_name, "BadCause");
        assert_eq!(
            cause
                .truncation
                .as_ref()
                .map_or(0, |marker| marker.failed_elements),
            1,
            "a descendant whose traceback read failed must report the loss"
        );
    });
}

#[rstest]
fn a_lookalike_traceback_yields_no_frames() {
    Python::attach(|py| {
        let error = py
            .run(
                c"def fail():\n    raise ValueError('fake traceback')\nfail()\n",
                None,
                None,
            )
            .expect_err("source should raise");
        let exc_value = error.value(py);

        // Control: the exception has a real traceback, and direct capture keeps
        // its frames. Without this step the test would pass if capture simply
        // ignored tracebacks altogether.
        let direct = capture_exception(py, exc_value)
            .expect("direct capture should succeed")
            .expect("payload should be present");
        assert!(
            !direct.frames.is_empty(),
            "the control exception must contribute real frames"
        );

        let exc_type = exc_value.get_type();
        // The lookalike's `tb_frame` is a property that counts reads, so a walker
        // that duck-typed the object would be caught even if it ignored the
        // missing `tb_lineno` and `tb_lasti` attributes.
        let globals = PyDict::new(py);
        py.run(
            c"
reads = 0

class Fake:
    @property
    def tb_frame(self):
        global reads
        reads += 1
        return None

    tb_next = None
",
            Some(&globals),
            None,
        )
        .expect("fake traceback class should build");
        let fake = globals
            .get_item("Fake")
            .expect("class lookup should succeed")
            .expect("class should be defined")
            .call0()
            .expect("fake traceback should build");
        let tuple = PyTuple::new(py, [exc_type.as_any(), exc_value, &fake])
            .expect("tuple creation should succeed");

        let payload = capture_exception(py, tuple.as_any())
            .expect("capture should not fail")
            .expect("payload should be present");

        // An `exc_info` tuple is caller-supplied, so its traceback may be any
        // object. One that is neither a real traceback nor a frame ends the
        // walk: it contributes no frames, and because nothing was dropped it is
        // not reported as an omission either.
        assert!(
            payload.frames.is_empty(),
            "an explicit override that is not a traceback must not be followed"
        );
        assert!(
            payload.truncation.is_none(),
            "a lookalike traceback should not be reported as omitted: {:?}",
            payload.truncation
        );
        let reads: usize = globals
            .get_item("reads")
            .expect("counter lookup should succeed")
            .expect("counter should be defined")
            .extract()
            .expect("counter should be an integer");
        assert_eq!(
            reads, 0,
            "type rejection must happen before any traceback attribute is read"
        );
    });
}
