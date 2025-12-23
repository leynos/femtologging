//! Python bindings for [`HTTPHandlerBuilder`].
//!
//! This module exposes Python APIs for constructing HTTP handlers with
//! URL configuration, authentication, timeouts, and serialisation options.

use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::handlers::HandlerBuilderTrait;
use crate::handlers::socket_builder::BackoffOverrides;
use crate::http_handler::{FemtoHTTPHandler, HTTPMethod};
use crate::macros::{AsPyDict, dict_into_py};

use super::HTTPHandlerBuilder;

#[pymethods]
impl HTTPHandlerBuilder {
    #[new]
    fn py_new() -> PyResult<Self> {
        Ok(Self::new())
    }

    #[pyo3(name = "with_url")]
    #[pyo3(signature = (url))]
    fn py_with_url<'py>(mut slf: PyRefMut<'py, Self>, url: String) -> PyRefMut<'py, Self> {
        let updated = slf.clone().with_url(url);
        *slf = updated;
        slf
    }

    #[pyo3(name = "with_method")]
    #[pyo3(signature = (method))]
    fn py_with_method<'py>(
        mut slf: PyRefMut<'py, Self>,
        method: &str,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let method = match method.to_uppercase().as_str() {
            "GET" => HTTPMethod::GET,
            "POST" => HTTPMethod::POST,
            _ => {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "unsupported HTTP method: {method}; expected GET or POST"
                )));
            }
        };
        let updated = slf.clone().with_method(method);
        *slf = updated;
        Ok(slf)
    }

    #[pyo3(name = "with_basic_auth")]
    #[pyo3(signature = (username, password))]
    fn py_with_basic_auth<'py>(
        mut slf: PyRefMut<'py, Self>,
        username: String,
        password: String,
    ) -> PyRefMut<'py, Self> {
        let updated = slf.clone().with_basic_auth(username, password);
        *slf = updated;
        slf
    }

    #[pyo3(name = "with_bearer_token")]
    #[pyo3(signature = (token))]
    fn py_with_bearer_token<'py>(
        mut slf: PyRefMut<'py, Self>,
        token: String,
    ) -> PyRefMut<'py, Self> {
        let updated = slf.clone().with_bearer_token(token);
        *slf = updated;
        slf
    }

    #[pyo3(name = "with_headers")]
    #[pyo3(signature = (headers))]
    fn py_with_headers<'py>(
        mut slf: PyRefMut<'py, Self>,
        headers: &Bound<'py, PyDict>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let mut headers_map = std::collections::HashMap::new();
        for (key, value) in headers.iter() {
            let key: String = key.extract()?;
            let value: String = value.extract()?;
            headers_map.insert(key, value);
        }
        let updated = slf.clone().with_headers(headers_map);
        *slf = updated;
        Ok(slf)
    }

    #[pyo3(name = "with_capacity")]
    #[pyo3(signature = (capacity))]
    fn py_with_capacity<'py>(mut slf: PyRefMut<'py, Self>, capacity: usize) -> PyRefMut<'py, Self> {
        let updated = slf.clone().with_capacity(capacity);
        *slf = updated;
        slf
    }

    #[pyo3(name = "with_connect_timeout_ms")]
    #[pyo3(signature = (timeout_ms))]
    fn py_with_connect_timeout<'py>(
        mut slf: PyRefMut<'py, Self>,
        timeout_ms: u64,
    ) -> PyRefMut<'py, Self> {
        let updated = slf.clone().with_connect_timeout_ms(timeout_ms);
        *slf = updated;
        slf
    }

    #[pyo3(name = "with_write_timeout_ms")]
    #[pyo3(signature = (timeout_ms))]
    fn py_with_write_timeout<'py>(
        mut slf: PyRefMut<'py, Self>,
        timeout_ms: u64,
    ) -> PyRefMut<'py, Self> {
        let updated = slf.clone().with_write_timeout_ms(timeout_ms);
        *slf = updated;
        slf
    }

    #[pyo3(name = "with_backoff")]
    fn py_with_backoff<'py>(
        mut slf: PyRefMut<'py, Self>,
        config: BackoffOverrides,
    ) -> PyRefMut<'py, Self> {
        let updated = slf.clone().with_backoff(config);
        *slf = updated;
        slf
    }

    #[pyo3(name = "with_json_format")]
    fn py_with_json_format<'py>(mut slf: PyRefMut<'py, Self>) -> PyRefMut<'py, Self> {
        let updated = slf.clone().with_json_format();
        *slf = updated;
        slf
    }

    #[pyo3(name = "with_record_fields")]
    #[pyo3(signature = (fields))]
    fn py_with_record_fields<'py>(
        mut slf: PyRefMut<'py, Self>,
        fields: Vec<String>,
    ) -> PyRefMut<'py, Self> {
        let updated = slf.clone().with_record_fields(fields);
        *slf = updated;
        slf
    }

    #[pyo3(name = "as_dict")]
    fn py_as_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let dict = PyDict::new(py);
        self.extend_dict(&dict)?;
        Ok(dict.into())
    }

    #[pyo3(name = "build")]
    fn py_build(&self) -> PyResult<FemtoHTTPHandler> {
        self.build_inner().map_err(Into::into)
    }
}

impl AsPyDict for HTTPHandlerBuilder {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let d = PyDict::new(py);
        self.extend_dict(&d)?;
        dict_into_py(d, py)
    }
}

#[cfg(test)]
mod tests {
    use pyo3::Python;
    use pyo3::types::{PyAnyMethods, PyDict, PyDictMethods};

    use crate::handlers::HandlerBuilderTrait;

    use super::HTTPHandlerBuilder;

    #[test]
    fn builder_requires_url() {
        Python::with_gil(|py| {
            let builder = pyo3::Py::new(py, HTTPHandlerBuilder::new())
                .expect("Py::new should succeed in test");
            let builder_ref = builder.borrow(py);
            let err = builder_ref
                .build_inner()
                .expect_err("build without URL should fail");
            assert!(err.to_string().contains("URL"));
        });
    }

    #[test]
    fn builder_with_url_succeeds() {
        Python::with_gil(|py| {
            let builder = HTTPHandlerBuilder::new().with_url("http://localhost:8080/log");
            let d = PyDict::new(py);
            builder
                .extend_dict(&d)
                .expect("dict serialisation succeeds");

            let url: String = d
                .get_item("url")
                .expect("get_item succeeds")
                .expect("url present")
                .extract()
                .expect("extract succeeds");
            assert_eq!(url, "http://localhost:8080/log");
        });
    }

    #[test]
    fn builder_with_json_format() {
        Python::with_gil(|py| {
            let builder = HTTPHandlerBuilder::new()
                .with_url("http://localhost:8080/log")
                .with_json_format();
            let d = PyDict::new(py);
            builder
                .extend_dict(&d)
                .expect("dict serialisation succeeds");

            let format: String = d
                .get_item("format")
                .expect("get_item succeeds")
                .expect("format present")
                .extract()
                .expect("extract succeeds");
            assert_eq!(format, "json");
        });
    }

    #[test]
    fn builder_with_basic_auth() {
        Python::with_gil(|py| {
            let builder = HTTPHandlerBuilder::new()
                .with_url("http://localhost:8080/log")
                .with_basic_auth("user", "pass");
            let d = PyDict::new(py);
            builder
                .extend_dict(&d)
                .expect("dict serialisation succeeds");

            let auth_type: String = d
                .get_item("auth_type")
                .expect("get_item succeeds")
                .expect("auth_type present")
                .extract()
                .expect("extract succeeds");
            assert_eq!(auth_type, "basic");

            let auth_user: String = d
                .get_item("auth_user")
                .expect("get_item succeeds")
                .expect("auth_user present")
                .extract()
                .expect("extract succeeds");
            assert_eq!(auth_user, "user");
        });
    }
}
