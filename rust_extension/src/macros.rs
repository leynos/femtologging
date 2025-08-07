use pyo3::{prelude::*, PyResult};

pub(crate) trait AsPyDict {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<PyObject>;
}

macro_rules! dict_set {
    ($py:ident, $dict:ident, $self:ident, opt $field:ident => $key:expr) => {
        if let Some(ref v) = $self.$field {
            $dict.set_item($key, v)?;
        }
    };
    ($py:ident, $dict:ident, $self:ident, opt_to_string $field:ident => $key:expr) => {
        if let Some(ref v) = $self.$field {
            $dict.set_item($key, v.to_string())?;
        }
    };
    ($py:ident, $dict:ident, $self:ident, vec $field:ident => $key:expr) => {
        if !$self.$field.is_empty() {
            $dict.set_item($key, &$self.$field)?;
        }
    };
    ($py:ident, $dict:ident, $self:ident, map $field:ident => $key:expr) => {
        if !$self.$field.is_empty() {
            let sub = pyo3::types::PyDict::new($py);
            for (k, v) in &$self.$field {
                sub.set_item(k, v.as_pydict($py)?)?;
            }
            $dict.set_item($key, sub)?;
        }
    };
    ($py:ident, $dict:ident, $self:ident, optmap $field:ident => $key:expr) => {
        if let Some(ref v) = $self.$field {
            $dict.set_item($key, v.as_pydict($py)?)?;
        }
    };
    ($py:ident, $dict:ident, $self:ident, val $field:ident => $key:expr) => {
        $dict.set_item($key, $self.$field)?;
    };
}
pub(crate) use dict_set;

macro_rules! impl_as_pydict {
    ($ty:ty { $( $kind:ident $field:ident => $key:expr ),* $(,)? }) => {
        impl AsPyDict for $ty {
            fn as_pydict(&self, py: Python<'_>) -> PyResult<PyObject> {
                let d = pyo3::types::PyDict::new(py);
                $(crate::macros::dict_set!(py, d, self, $kind $field => $key);)*
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
