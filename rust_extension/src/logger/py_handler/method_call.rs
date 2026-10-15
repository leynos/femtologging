//! Typed call metadata for invoking Python logger handler methods.

use pyo3::{Bound, PyAny};

pub(super) struct DirectMethodSpec {
    pub(super) name: &'static str,
    pub(super) lookup_operation: &'static str,
    pub(super) call_operation: &'static str,
}

pub(super) struct ContextMethodCall<'py> {
    pub(super) method_caller: Bound<'py, PyAny>,
    pub(super) operation: &'static str,
}

impl<'py> ContextMethodCall<'py> {
    pub(super) fn new(method_caller: Bound<'py, PyAny>, operation: &'static str) -> Self {
        Self {
            method_caller,
            operation,
        }
    }
}
