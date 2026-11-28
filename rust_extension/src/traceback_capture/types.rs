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
use pyo3::types::{PyTraceback, PyTuple, PyType};

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
    inherits_native_method(value, py.get_type::<PyBaseException>().as_any(), "__repr__")
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
    inherits_native_method(value, py.get_type::<PyBaseException>().as_any(), "__str__")
}

/// Return `true` when `value` inherits `object.__str__`.
///
/// Only then does `str(value)` fall back to `repr`, which the bounded
/// container renderer reproduces. A subclass with its own `__str__` keeps that
/// representation instead.
pub(super) fn inherits_object_str(value: &Bound<'_, PyAny>) -> bool {
    let py = value.py();
    inherits_native_method(value, py.get_type::<PyAny>().as_any(), "__str__")
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
    let Some(inherited) = method_definition(native, method) else {
        return false;
    };
    let Some(actual) = method_definition(value.get_type().as_any(), method) else {
        return false;
    };
    actual.is(&inherited)
}

/// Return the first definition of `name` along a type's MRO.
///
/// Reading `cls.__str__` through `type.__getattribute__` still honours a
/// metaclass data descriptor, which can answer for the class instead of the
/// class's own MRO. Walking the MRO's raw dictionaries is how CPython's special
/// method lookup resolves `__str__` and `__repr__`, so a metaclass property
/// cannot fake an inherited definition.
fn method_definition<'py>(class: &Bound<'py, PyAny>, name: &str) -> Option<Bound<'py, PyAny>> {
    let py = class.py();
    let type_type = py.get_type::<PyType>();
    let getattribute = type_type.getattr("__getattribute__").ok()?;
    let mro = getattribute.call1((class, "__mro__")).ok()?;
    let mro = mro.cast::<PyTuple>().ok()?;
    for base in mro.iter() {
        let dictionary = getattribute.call1((&base, "__dict__")).ok()?;
        // A type's `__dict__` is a mapping proxy, not a dict, so it is queried
        // through its own `get`.
        let value = dictionary.call_method1("get", (name,)).ok()?;
        if !value.is_none() {
            return Some(value);
        }
    }
    None
}

/// Check the value and traceback positions of an `exc_info` triple.
///
/// The checks happen on every path, including `validate_exc_info`, so an
/// invalid tuple is rejected before logger filters can drop the record and hide
/// the programming error. Only types are inspected; capture itself stays
/// deferred.
pub(super) fn validate_tuple_members(py: Python<'_>, tuple: &Bound<'_, PyTuple>) -> PyResult<()> {
    let value = tuple.get_item(1)?;
    if !value.is_none() && !is_exception(py, &value) {
        return Err(pyo3::exceptions::PyTypeError::new_err(
            "exc_info tuple value must be an exception instance or None",
        ));
    }
    let traceback = tuple.get_item(2)?;
    if !traceback.is_none() && traceback.cast::<PyTraceback>().is_err() {
        return Err(pyo3::exceptions::PyTypeError::new_err(
            "exc_info tuple traceback must be a traceback or None",
        ));
    }
    Ok(())
}
