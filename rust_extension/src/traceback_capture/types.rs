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

/// Read a class attribute through `type.__getattribute__`.
fn class_attribute<'py>(class: &Bound<'py, PyAny>, name: &str) -> Option<Bound<'py, PyAny>> {
    let py = class.py();
    let type_type = py.get_type::<PyType>();
    let getattribute = type_type.getattr("__getattribute__").ok()?;
    getattribute.call1((class, name)).ok()
}
