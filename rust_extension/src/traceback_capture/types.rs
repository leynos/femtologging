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
