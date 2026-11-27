//! Helpers for classifying real Python exception objects.
//!
//! An exception subclass can override `__class__` or expose a lying `args`
//! property, so classification asks the object's *real* type — `PyObject_IsSubclass`
//! on `get_type()` — rather than `isinstance`, which consults `__class__`.
//!
//! The comparison types come from PyO3's native exception objects rather than
//! from `builtins` lookups, so rebinding `builtins.BaseException` or
//! `builtins.BaseExceptionGroup` cannot change what counts as an exception or a
//! group.

use pyo3::exceptions::{PyBaseException, PyBaseExceptionGroup};
use pyo3::prelude::*;
use pyo3::types::PyType;

/// Return `true` when `value` is a real Python exception instance.
pub(super) fn is_exception(py: Python<'_>, value: &Bound<'_, PyAny>) -> bool {
    value
        .get_type()
        .is_subclass(&py.get_type::<PyBaseException>())
        .unwrap_or(false)
}

/// Return `true` when `exception` is an exception group.
///
/// The project supports Python 3.12 and later, so the native group type always
/// exists and no runtime feature probe is needed.
pub(super) fn is_exception_group(py: Python<'_>, exception: &Bound<'_, PyAny>) -> bool {
    exception
        .get_type()
        .is_subclass(&py.get_type::<PyBaseExceptionGroup>())
        .unwrap_or(false)
}
/// Return `true` when `value` uses the built-in `BaseException.__repr__`.
///
/// That representation renders every argument in full before any byte ceiling
/// can apply. Types that define their own `__repr__` are left to it: their
/// representation is what the application chose to expose.
pub(super) fn inherits_exception_repr(value: &Bound<'_, PyAny>) -> bool {
    let py = value.py();
    if !is_exception(py, value) {
        return false;
    }
    let base = py.get_type::<PyBaseException>();
    let Some(inherited) = class_attribute(base.as_any(), "__repr__") else {
        return false;
    };
    let Some(actual) = class_attribute(value.get_type().as_any(), "__repr__") else {
        return false;
    };
    actual.is(&inherited)
}

/// Return `true` when `value` uses the built-in `BaseException.__str__`.
///
/// That implementation renders the argument tuple, so a nested exception can
/// build an unbounded message through it. Types with their own `__str__` keep
/// their chosen representation.
pub(super) fn inherits_exception_str(value: &Bound<'_, PyAny>) -> bool {
    let py = value.py();
    if !is_exception(py, value) {
        return false;
    }
    let base = py.get_type::<PyBaseException>();
    let Some(inherited) = class_attribute(base.as_any(), "__str__") else {
        return false;
    };
    let Some(actual) = class_attribute(value.get_type().as_any(), "__str__") else {
        return false;
    };
    actual.is(&inherited)
}

/// Return `true` when `value` inherits `object.__str__`.
///
/// Only then does `str(value)` fall back to `repr`, which the bounded
/// container renderer reproduces. A subclass with its own `__str__` keeps that
/// representation instead.
pub(super) fn inherits_object_str(value: &Bound<'_, PyAny>) -> bool {
    let py = value.py();
    let object = py.get_type::<PyAny>();
    let Some(inherited) = class_attribute(object.as_any(), "__str__") else {
        return false;
    };
    let Some(actual) = class_attribute(value.get_type().as_any(), "__str__") else {
        return false;
    };
    actual.is(&inherited)
}

/// Return `true` when `value` inherits `method` unchanged from `native`.
///
/// Used to decide whether a built-in type's own representation applies: an
/// inherited method can be replaced by the bounded renderer, while a genuine
/// override must keep the representation the application defined.
pub(super) fn inherits_native_method(
    value: &Bound<'_, PyAny>,
    native: &Bound<'_, PyAny>,
    method: &str,
) -> bool {
    let Some(inherited) = class_attribute(native, method) else {
        return false;
    };
    let Some(actual) = class_attribute(value.get_type().as_any(), method) else {
        return false;
    };
    actual.is(&inherited)
}

/// Read a class attribute through `type.__getattribute__`.
fn class_attribute<'py>(class: &Bound<'py, PyAny>, name: &str) -> Option<Bound<'py, PyAny>> {
    let py = class.py();
    let type_type = py.get_type::<PyType>();
    let getattribute = type_type.getattr("__getattribute__").ok()?;
    getattribute.call1((class, name)).ok()
}
