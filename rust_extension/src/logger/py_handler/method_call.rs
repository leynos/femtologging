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
    pub(super) const fn new(method_caller: Bound<'py, PyAny>, operation: &'static str) -> Self {
        Self {
            method_caller,
            operation,
        }
    }
}

#[cfg(test)]
mod const_eval_tests {
    //! Compile-time pin for [`ContextMethodCall::new`].
    //!
    //! The constructor is `const` so a call site can build it where the caller
    //! is declared. A `Bound<PyAny>` cannot be produced in a const item, so the
    //! constructor is reached through a `const fn` wrapper that takes it as a
    //! parameter: a `const fn` body is checked for const-evaluability at
    //! definition time, so this stops compiling if the qualifier is lost.
    //!
    //! Binding the function pointer would not do — it accepts a non-`const`
    //! `fn`, so it pins the signature and not the guarantee.

    use pyo3::prelude::*;

    use super::ContextMethodCall;

    const fn build<'py>(
        method_caller: Bound<'py, PyAny>,
        operation: &'static str,
    ) -> ContextMethodCall<'py> {
        ContextMethodCall::new(method_caller, operation)
    }

    const _: fn(Bound<'static, PyAny>, &'static str) -> ContextMethodCall<'static> = build;
}
