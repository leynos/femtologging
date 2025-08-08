use pyo3::conversion::IntoPyObject;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
    Bound, PyResult,
};
use std::collections::BTreeMap;

pub(crate) trait AsPyDict {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<PyObject>;
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
    T: IntoPyObject<'py> + Clone,
{
    dict.set_item(key, val.clone())
}

macro_rules! impl_as_pydict {
    ($ty:ty { $( $setter:ident $field:ident => $key:expr ),* $(,)? }) => {
        impl AsPyDict for $ty {
            fn as_pydict(&self, py: Python<'_>) -> PyResult<PyObject> {
                let d = pyo3::types::PyDict::new(py);
                $(crate::macros::$setter(py, &d, $key, &self.$field)?;)*
                Ok(d.into())
            }
        }
    };
}
pub(crate) use impl_as_pydict;

macro_rules! py_setters {
    ($builder:ident { $( $field:ident : $fname:ident => $py_name:literal, $ty:ty, $conv:expr, $doc:literal ),* $(,)? }) => {
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

            fn as_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
                self.as_pydict(py)
            }
        }
    };
}
pub(crate) use py_setters;
