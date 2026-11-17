//! Helpers for building real Python exceptions in traceback capture tests.

use pyo3::prelude::*;

/// Look up a name in the `builtins` module, such as an exception type.
pub fn builtin_type<'py>(py: Python<'py>, type_name: &str) -> PyResult<Bound<'py, PyAny>> {
    py.import("builtins")?.getattr(type_name)
}

/// Create a built-in exception instance from `type_name` with the given arguments.
pub fn create_builtin_exception<'py>(
    py: Python<'py>,
    type_name: &str,
    args: impl pyo3::call::PyCallArgs<'py>,
) -> PyResult<Bound<'py, PyAny>> {
    builtin_type(py, type_name)?.call1(args)
}

/// Create a `ValueError` instance carrying the given message.
pub fn create_value_error<'py>(py: Python<'py>, message: &str) -> PyResult<Bound<'py, PyAny>> {
    create_builtin_exception(py, "ValueError", (message,))
}

/// Create a `BaseException` instance with no arguments.
pub fn create_base_exception<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
    builtin_type(py, "BaseException")?.call0()
}
