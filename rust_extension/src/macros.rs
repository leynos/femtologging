//! Macros and traits for converting Rust structs to Python dictionaries.
#![cfg_attr(not(feature = "python"), allow(dead_code, unused_imports))]
//!
//! This module provides the `AsPyDict` trait and associated macros to generate
//! consistent Python dictionary representations of configuration builder
//! structs. The macros reduce boilerplate in PyO3 bindings whilst ensuring
//! uniform serialization behaviour across all builder types.

use pyo3::conversion::IntoPyObject;
use pyo3::IntoPyObjectExt;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
    Bound, PyResult,
};
use std::collections::BTreeMap;

/// Convert a configuration builder into a Python dictionary.
pub trait AsPyDict {
    /// Return the builder's state as a Python dictionary.
    fn as_pydict(&self, py: Python<'_>) -> PyResult<PyObject>;
}

/// Convert a [`PyDict`] bound to the current GIL into a [`PyObject`].
///
/// # Example
///
/// ```rust,ignore
/// use crate::macros::dict_into_py;
/// use pyo3::prelude::*;
///
/// fn demo(py: Python<'_>) -> PyResult<()> {
///     let d = pyo3::types::PyDict::new(py);
///     let obj = dict_into_py(d, py)?;
///     let _ = obj;
///     Ok(())
/// }
/// ```
pub(crate) fn dict_into_py(dict: Bound<'_, PyDict>, py: Python<'_>) -> PyResult<PyObject> {
    dict.into_py_any(py)
}

pub(crate) fn set_opt<'py, T>(
    _py: Python<'py>,
    dict: &Bound<'py, PyDict>,
    key: &str,
    opt: &Option<T>,
) -> PyResult<()>
where
    T: IntoPyObject<'py> + Clone,
{
    if let Some(v) = opt {
        dict.set_item(key, v.clone())?;
    }
    Ok(())
}

pub(crate) fn set_opt_to_string<T: ToString>(
    _py: Python<'_>,
    dict: &Bound<'_, PyDict>,
    key: &str,
    opt: &Option<T>,
) -> PyResult<()> {
    if let Some(v) = opt {
        dict.set_item(key, v.to_string())?;
    }
    Ok(())
}

pub(crate) fn set_vec<'py, T>(
    py: Python<'py>,
    dict: &Bound<'py, PyDict>,
    key: &str,
    vec: &[T],
) -> PyResult<()>
where
    T: IntoPyObject<'py> + Clone,
{
    if !vec.is_empty() {
        // Convert the slice to a Python list only when non-empty.
        // `PyList::new` surfaces element conversion failures via its
        // `PyResult` return type.
        let list = PyList::new(py, vec.iter().cloned())?;
        dict.set_item(key, list)?;
    }
    Ok(())
}

pub(crate) fn set_map<V: AsPyDict>(
    py: Python<'_>,
    dict: &Bound<'_, PyDict>,
    key: &str,
    map: &BTreeMap<String, V>,
) -> PyResult<()> {
    if !map.is_empty() {
        let sub = PyDict::new(py);
        for (k, v) in map {
            sub.set_item(k, v.as_pydict(py)?)?;
        }
        dict.set_item(key, sub)?;
    }
    Ok(())
}

pub(crate) fn set_optmap<V: AsPyDict>(
    py: Python<'_>,
    dict: &Bound<'_, PyDict>,
    key: &str,
    opt: &Option<V>,
) -> PyResult<()> {
    if let Some(v) = opt {
        dict.set_item(key, v.as_pydict(py)?)?;
    }
    Ok(())
}

pub(crate) fn set_val<'py, T>(
    _py: Python<'py>,
    dict: &Bound<'py, PyDict>,
    key: &str,
    val: &T,
) -> PyResult<()>
where
    T: IntoPyObject<'py> + Copy,
{
    dict.set_item(key, *val)
}

macro_rules! impl_as_pydict {
    ($ty:ty { $( $setter:ident $field:ident => $key:expr ),* $(,)? }) => {
        impl AsPyDict for $ty {
            fn as_pydict(&self, py: Python<'_>) -> PyResult<PyObject> {
                let d = pyo3::types::PyDict::new(py);
                $(crate::macros::$setter(py, &d, $key, &self.$field)?;)*
                crate::macros::dict_into_py(d, py)
            }
        }
    };
}
pub(crate) use impl_as_pydict;

macro_rules! py_setters {
    (
        $builder:ident {
            $( $field:ident : $fname:ident => $py_name:literal, $ty:ty, $conv:expr, $doc:literal ),* $(,)?
        }
        $(; $($extra:item)*)?
    ) => {
        #[pymethods]
        impl $builder {
            #[new]
            fn py_new() -> Self { Self::new() }

            $(
            #[doc = $doc]
            #[pyo3(name = $py_name)]
            fn $fname<'py>(mut slf: PyRefMut<'py, Self>, val: $ty) -> PyRefMut<'py, Self> {
                slf.$field = $conv(val);
                slf
            }
            )*

            $($($extra)*)*

            fn as_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
                self.as_pydict(py)
            }
        }
    };
}
pub(crate) use py_setters;
