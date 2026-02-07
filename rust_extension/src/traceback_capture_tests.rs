//! Unit tests for traceback capture utilities.
//!
//! Graceful degradation tests (missing attributes, malformed `__notes__`,
//! chained exceptions) are in [`crate::traceback_capture_graceful_degradation_tests`].

use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyTuple};
use rstest::rstest;

use crate::exception_schema::EXCEPTION_SCHEMA_VERSION;
use crate::traceback_capture::{capture_exception, capture_stack};

#[rstest]
fn capture_exception_with_true_no_active_exception() {
    Python::attach(|py| {
        let true_val = PyBool::new(py, true);
        let result = capture_exception(py, true_val.as_any())
            .expect("capture_exception should not fail with True");
        assert!(result.is_none(), "No active exception should return None");
    });
}

#[rstest]
fn capture_exception_with_false_returns_none() {
    Python::attach(|py| {
        let false_val = PyBool::new(py, false);
        let result = capture_exception(py, false_val.as_any())
            .expect("capture_exception should not fail with False");
        assert!(result.is_none());
    });
}

#[rstest]
fn capture_exception_with_instance() {
    Python::attach(|py| {
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
    Python::attach(|py| {
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
    Python::attach(|py| {
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
    Python::attach(|py| {
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
    Python::attach(|py| {
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
    Python::attach(|py| {
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
    Python::attach(|py| {
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
    Python::attach(|py| {
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
    Python::attach(|py| {
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
fn capture_exception_builtin_has_no_module() {
    // Built-in exceptions should have module=None because "builtins" is filtered.
    Python::with_gil(|py| {
        let exc = py
            .import("builtins")
            .expect("builtins module should exist")
            .getattr("ValueError")
            .expect("ValueError should exist")
            .call1(("test",))
            .expect("ValueError constructor should succeed");

        let payload = capture_exception(py, &exc)
            .expect("capture_exception should succeed")
            .expect("payload should be Some");

        assert_eq!(payload.type_name, "ValueError");
        assert!(
            payload.module.is_none(),
            "built-in exceptions should have module=None, got: {:?}",
            payload.module
        );
    });
}

#[rstest]
fn capture_exception_main_module_preserves_module() {
    // Exceptions from __main__ should retain their module value. Only
    // "builtins" is filtered; "__main__" is meaningful metadata for
    // user-defined exceptions in scripts and entry points.
    Python::with_gil(|py| {
        let code = c"exc = type('MainError', (Exception,), {'__module__': '__main__'})('test')";
        let globals = PyDict::new(py);
        py.run(code, Some(&globals), None)
            .expect("code to create __main__ exception should succeed");

        let exc = globals
            .get_item("exc")
            .expect("get_item should not fail")
            .expect("exc should exist");

        let payload = capture_exception(py, &exc)
            .expect("capture_exception should succeed")
            .expect("payload should be Some");

        assert_eq!(payload.type_name, "MainError");
        assert_eq!(
            payload.module.as_deref(),
            Some("__main__"),
            "__main__ exceptions should preserve their module"
        );
    });
}

#[rstest]
fn capture_exception_custom_module_has_module() {
    // Exceptions from non-builtin modules should include the module name.
    Python::with_gil(|py| {
        let code = c"
import types, sys

mod = types.ModuleType('custom_mod')
class CustomError(Exception):
    pass
CustomError.__module__ = 'custom_mod'
mod.CustomError = CustomError
sys.modules['custom_mod'] = mod

exc = CustomError('test')
";
        let globals = PyDict::new(py);
        py.run(code, Some(&globals), None)
            .expect("code to create custom exception should succeed");

        let exc = globals
            .get_item("exc")
            .expect("get_item should not fail")
            .expect("exc should exist");

        let payload = capture_exception(py, &exc)
            .expect("capture_exception should succeed")
            .expect("payload should be Some");

        assert_eq!(payload.type_name, "CustomError");
        assert_eq!(
            payload.module.as_deref(),
            Some("custom_mod"),
            "custom module exception should report its module"
        );
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
