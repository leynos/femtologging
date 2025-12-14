//! Python bindings for [`SocketHandlerBuilder`].

use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::macros::{AsPyDict, dict_into_py};
use crate::socket_handler::FemtoSocketHandler;

use super::{BackoffOverrides, HandlerBuilderTrait, SocketHandlerBuilder};

fn extract_optional_u64<'py>(config: &Bound<'py, PyDict>, key: &str) -> PyResult<Option<u64>> {
    match config.get_item(key)? {
        None => Ok(None),
        Some(value) if value.is_none() => Ok(None),
        Some(value) => value.extract::<u64>().map(Some),
    }
}

#[pymethods]
impl BackoffOverrides {
    #[new]
    #[pyo3(signature = (config=None))]
    fn py_new<'py>(config: Option<Bound<'py, PyDict>>) -> PyResult<Self> {
        let config = match config {
            Some(dict) => dict,
            None => return Ok(Self::default()),
        };

        // Fail fast on typos / unsupported keys.
        const ALLOWED_KEYS: [&str; 4] = ["base_ms", "cap_ms", "reset_after_ms", "deadline_ms"];
        for (key, _) in config.iter() {
            let key: &str = key.extract()?;
            if !ALLOWED_KEYS.contains(&key) {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "unknown BackoffConfig key {key:?}",
                )));
            }
        }

        let base_ms = extract_optional_u64(&config, "base_ms")?;
        let cap_ms = extract_optional_u64(&config, "cap_ms")?;
        let reset_after_ms = extract_optional_u64(&config, "reset_after_ms")?;
        let deadline_ms = extract_optional_u64(&config, "deadline_ms")?;

        Ok(Self::from_options(
            base_ms,
            cap_ms,
            reset_after_ms,
            deadline_ms,
        ))
    }
}

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
    fn py_with_backoff<'py>(
        mut slf: PyRefMut<'py, Self>,
        config: BackoffOverrides,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let updated = slf.clone().with_backoff(config);
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
