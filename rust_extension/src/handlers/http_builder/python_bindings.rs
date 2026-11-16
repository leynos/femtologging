//! Python bindings for [`HTTPHandlerBuilder`].
//!
//! This module exposes Python APIs for constructing HTTP handlers with
//! URL configuration, authentication, timeouts, and serialization options.

use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::handlers::HandlerBuilderTrait;
use crate::handlers::socket_builder::BackoffOverrides;
use crate::http_handler::{FemtoHTTPHandler, HTTPMethod};
use crate::macros::{AsPyDict, dict_into_py};

use super::HTTPHandlerBuilder;

mod auth;

use auth::{AuthSpec, parse_auth_spec};

/// Maintains handler construction and delivery contracts so configuration is validated before asynchronous runtime state is published.
fn parse_http_method(method: &str) -> PyResult<HTTPMethod> {
    match method.to_uppercase().as_str() {
        "GET" => Ok(HTTPMethod::GET),
        "POST" => Ok(HTTPMethod::POST),
        _ => Err(pyo3::exceptions::PyValueError::new_err(format!(
            "unsupported HTTP method: {method}; expected GET or POST"
        ))),
    }
}

#[pymethods]
impl HTTPHandlerBuilder {
    /// Maintains handler construction and delivery contracts so configuration is validated before asynchronous runtime state is published.
    #[new]
    fn py_new() -> Self {
        Self::new()
    }

    /// Maintains handler construction and delivery contracts so configuration is validated before asynchronous runtime state is published.
    #[pyo3(name = "with_url")]
    #[pyo3(signature = (url))]
    fn py_with_url(mut slf: PyRefMut<'_, Self>, url: String) -> PyRefMut<'_, Self> {
        let updated = slf.clone().with_url(url);
        *slf = updated;
        slf
    }

    /// Maintains handler construction and delivery contracts so configuration is validated before asynchronous runtime state is published.
    #[pyo3(name = "with_method")]
    #[pyo3(signature = (method))]
    fn py_with_method<'py>(
        mut slf: PyRefMut<'py, Self>,
        method: &str,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let parsed_method = parse_http_method(method)?;
        let updated = slf.clone().with_method(parsed_method);
        *slf = updated;
        Ok(slf)
    }

    /// Configure the HTTP endpoint URL and optional request method.
    #[pyo3(name = "with_endpoint")]
    #[pyo3(signature = (url, method = None))]
    fn py_with_endpoint(
        mut slf: PyRefMut<'_, Self>,
        url: String,
        method: Option<String>,
    ) -> PyResult<PyRefMut<'_, Self>> {
        let updated = if let Some(m) = method {
            let parsed_method = parse_http_method(&m)?;
            slf.clone().with_url(url).with_method(parsed_method)
        } else {
            slf.clone().with_url(url)
        };
        *slf = updated;
        Ok(slf)
    }

    /// Configure HTTP authentication from a Python mapping.
    #[pyo3(name = "with_auth")]
    #[pyo3(signature = (config))]
    fn py_with_auth<'py>(
        mut slf: PyRefMut<'py, Self>,
        config: &Bound<'py, PyDict>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let updated = match parse_auth_spec(config)? {
            AuthSpec::Bearer(token) => slf.clone().with_bearer_token(token),
            AuthSpec::Basic { username, password } => {
                slf.clone().with_basic_auth(username, password)
            }
        };
        *slf = updated;
        Ok(slf)
    }

    /// Maintains handler construction and delivery contracts so configuration is validated before asynchronous runtime state is published.
    #[pyo3(name = "with_basic_auth")]
    #[pyo3(signature = (username, password))]
    fn py_with_basic_auth(
        mut slf: PyRefMut<'_, Self>,
        username: String,
        password: String,
    ) -> PyRefMut<'_, Self> {
        let updated = slf.clone().with_basic_auth(username, password);
        *slf = updated;
        slf
    }

    /// Maintains handler construction and delivery contracts so configuration is validated before asynchronous runtime state is published.
    #[pyo3(name = "with_bearer_token")]
    #[pyo3(signature = (token))]
    fn py_with_bearer_token(mut slf: PyRefMut<'_, Self>, token: String) -> PyRefMut<'_, Self> {
        let updated = slf.clone().with_bearer_token(token);
        *slf = updated;
        slf
    }

    /// Maintains handler construction and delivery contracts so configuration is validated before asynchronous runtime state is published.
    #[pyo3(name = "with_headers")]
    #[pyo3(signature = (headers))]
    fn py_with_headers<'py>(
        mut slf: PyRefMut<'py, Self>,
        headers: &Bound<'py, PyDict>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let mut headers_map = std::collections::HashMap::new();
        for (key_obj, value_obj) in headers.iter() {
            let key: String = key_obj.extract()?;
            let value: String = value_obj.extract()?;
            headers_map.insert(key, value);
        }
        let updated = slf.clone().with_headers(headers_map);
        *slf = updated;
        Ok(slf)
    }

    /// Maintains handler construction and delivery contracts so configuration is validated before asynchronous runtime state is published.
    #[pyo3(name = "with_capacity")]
    #[pyo3(signature = (capacity))]
    fn py_with_capacity(mut slf: PyRefMut<'_, Self>, capacity: usize) -> PyRefMut<'_, Self> {
        let updated = slf.clone().with_capacity(capacity);
        *slf = updated;
        slf
    }

    /// Maintains handler construction and delivery contracts so configuration is validated before asynchronous runtime state is published.
    #[pyo3(name = "with_connect_timeout_ms")]
    #[pyo3(signature = (timeout_ms))]
    fn py_with_connect_timeout(mut slf: PyRefMut<'_, Self>, timeout_ms: u64) -> PyRefMut<'_, Self> {
        let updated = slf.clone().with_connect_timeout_ms(timeout_ms);
        *slf = updated;
        slf
    }

    /// Maintains handler construction and delivery contracts so configuration is validated before asynchronous runtime state is published.
    #[pyo3(name = "with_write_timeout_ms")]
    #[pyo3(signature = (timeout_ms))]
    fn py_with_write_timeout(mut slf: PyRefMut<'_, Self>, timeout_ms: u64) -> PyRefMut<'_, Self> {
        let updated = slf.clone().with_write_timeout_ms(timeout_ms);
        *slf = updated;
        slf
    }

    /// Maintains handler construction and delivery contracts so configuration is validated before asynchronous runtime state is published.
    #[pyo3(name = "with_backoff")]
    fn py_with_backoff(
        mut slf: PyRefMut<'_, Self>,
        config: BackoffOverrides,
    ) -> PyRefMut<'_, Self> {
        let updated = slf.clone().with_backoff(config);
        *slf = updated;
        slf
    }

    /// Maintains handler construction and delivery contracts so configuration is validated before asynchronous runtime state is published.
    #[pyo3(name = "with_json_format")]
    fn py_with_json_format(mut slf: PyRefMut<'_, Self>) -> PyRefMut<'_, Self> {
        let updated = slf.clone().with_json_format();
        *slf = updated;
        slf
    }

    /// Maintains handler construction and delivery contracts so configuration is validated before asynchronous runtime state is published.
    #[pyo3(name = "with_record_fields")]
    #[pyo3(signature = (fields))]
    fn py_with_record_fields(
        mut slf: PyRefMut<'_, Self>,
        fields: Vec<String>,
    ) -> PyRefMut<'_, Self> {
        let updated = slf.clone().with_record_fields(fields);
        *slf = updated;
        slf
    }

    /// Attach filters to this handler by configured filter identifier.
    #[pyo3(name = "with_filters")]
    fn py_with_filters(mut slf: PyRefMut<'_, Self>, filter_ids: Vec<String>) -> PyRefMut<'_, Self> {
        let updated = slf.clone().with_filters(filter_ids);
        *slf = updated;
        slf
    }

    /// Maintains handler construction and delivery contracts so configuration is validated before asynchronous runtime state is published.
    #[pyo3(name = "as_dict")]
    fn py_as_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let dict = PyDict::new(py);
        self.extend_dict(&dict)?;
        Ok(dict.into())
    }

    /// Maintains handler construction and delivery contracts so configuration is validated before asynchronous runtime state is published.
    #[pyo3(name = "build")]
    fn py_build(&self) -> PyResult<FemtoHTTPHandler> {
        self.build_inner().map_err(Into::into)
    }
}

impl AsPyDict for HTTPHandlerBuilder {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let d = PyDict::new(py);
        self.extend_dict(&d)?;
        dict_into_py(d, py)
    }
}

#[cfg(test)]
#[path = "python_bindings_tests.rs"]
mod tests;
