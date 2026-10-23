//! Typed call metadata for invoking Python logger handler methods.

use pyo3::{Bound, PyAny};

/// Names and diagnostic labels for one direct Python handler method call.
pub(super) struct DirectMethodSpec {
    /// Python attribute to look up on the wrapped handler object.
    pub(super) name: &'static str,
    /// Operation label used when looking up the method fails.
    pub(super) lookup_operation: &'static str,
    /// Operation label used when invoking the method fails.
    pub(super) call_operation: &'static str,
}

/// Prepared Python `methodcaller` and its operation label for context execution.
pub(super) struct ContextMethodCall<'py> {
    /// Owned callable bound to the Python interpreter that will run the method.
    pub(super) method_caller: Bound<'py, PyAny>,
    /// Operation label used when the context-bound invocation fails.
    pub(super) operation: &'static str,
}

impl<'py> ContextMethodCall<'py> {
    /// Pairs a prepared method caller with the operation name used in errors.
    pub(super) fn new(method_caller: Bound<'py, PyAny>, operation: &'static str) -> Self {
        Self {
            method_caller,
            operation,
        }
    }
}
