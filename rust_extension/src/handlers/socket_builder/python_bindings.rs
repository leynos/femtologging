//! Python bindings for [`SocketHandlerBuilder`].

use pyo3::{prelude::*, types::PyDict};

use crate::macros::{dict_into_py, AsPyDict};
use crate::socket_handler::FemtoSocketHandler;

use super::{BackoffOverrides, HandlerBuilderTrait, SocketHandlerBuilder};

#[pymethods]
impl SocketHandlerBuilder {
    #[new]
    fn py_new() -> PyResult<Self> {
        Ok(Self::new())
    }

    #[pyo3(name = "with_tcp")]
    #[pyo3(signature = (host, port))]
    fn py_with_tcp<'py>(
        mut slf: PyRefMut<'py, Self>,
        host: String,
        port: u16,
    ) -> PyRefMut<'py, Self> {
        let updated = slf.clone().with_tcp(host, port);
        *slf = updated;
        slf
    }

    #[pyo3(name = "with_unix_path")]
    #[pyo3(signature = (path))]
    fn py_with_unix_path<'py>(mut slf: PyRefMut<'py, Self>, path: String) -> PyRefMut<'py, Self> {
        let updated = slf.clone().with_unix_path(path);
        *slf = updated;
        slf
    }

    #[pyo3(name = "with_capacity")]
    #[pyo3(signature = (capacity))]
    fn py_with_capacity<'py>(
        mut slf: PyRefMut<'py, Self>,
        capacity: usize,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let updated = slf.clone().with_capacity(capacity);
        *slf = updated;
        Ok(slf)
    }

    #[pyo3(name = "with_connect_timeout_ms")]
    #[pyo3(signature = (timeout_ms))]
    fn py_with_connect_timeout<'py>(
        mut slf: PyRefMut<'py, Self>,
        timeout_ms: u64,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let updated = slf.clone().with_connect_timeout_ms(timeout_ms);
        *slf = updated;
        Ok(slf)
    }

    #[pyo3(name = "with_write_timeout_ms")]
    #[pyo3(signature = (timeout_ms))]
    fn py_with_write_timeout<'py>(
        mut slf: PyRefMut<'py, Self>,
        timeout_ms: u64,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let updated = slf.clone().with_write_timeout_ms(timeout_ms);
        *slf = updated;
        Ok(slf)
    }

    #[pyo3(name = "with_max_frame_size")]
    #[pyo3(signature = (size))]
    fn py_with_max_frame_size<'py>(
        mut slf: PyRefMut<'py, Self>,
        size: u64,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let size = usize::try_from(size).map_err(|_| {
            pyo3::exceptions::PyOverflowError::new_err(
                "max_frame_size does not fit in platform usize",
            )
        })?;
        let updated = slf.clone().with_max_frame_size(size);
        *slf = updated;
        Ok(slf)
    }

    #[pyo3(name = "with_tls")]
    #[pyo3(signature = (domain=None, *, insecure=false))]
    fn py_with_tls<'py>(
        mut slf: PyRefMut<'py, Self>,
        domain: Option<String>,
        insecure: bool,
    ) -> PyRefMut<'py, Self> {
        let updated = slf.clone().with_tls(domain, insecure);
        *slf = updated;
        slf
    }

    #[pyo3(name = "with_backoff")]
    #[pyo3(signature = (base_ms=None, cap_ms=None, reset_after_ms=None, deadline_ms=None))]
    fn py_with_backoff<'py>(
        mut slf: PyRefMut<'py, Self>,
        base_ms: Option<u64>,
        cap_ms: Option<u64>,
        reset_after_ms: Option<u64>,
        deadline_ms: Option<u64>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let overrides =
            BackoffOverrides::from_options(base_ms, cap_ms, reset_after_ms, deadline_ms);
        let updated = slf.clone().with_backoff(overrides);
        *slf = updated;
        Ok(slf)
    }

    #[pyo3(name = "as_dict")]
    fn py_as_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let dict = PyDict::new(py);
        self.extend_dict(&dict)?;
        Ok(dict.into())
    }

    #[pyo3(name = "build")]
    fn py_build(&self) -> PyResult<FemtoSocketHandler> {
        self.build_inner().map_err(Into::into)
    }
}

impl AsPyDict for SocketHandlerBuilder {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let d = PyDict::new(py);
        self.extend_dict(&d)?;
        dict_into_py(d, py)
    }
}
