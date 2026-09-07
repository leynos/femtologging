//! Python bindings for [`HTTPHandlerBuilder`].
//!
//! This module exposes Python APIs for constructing HTTP handlers with
//! URL configuration, authentication, timeouts, and serialization options.

#![expect(
    clippy::too_many_arguments,
    reason = "PyO3 generates five-argument Python call wrappers"
)]

use pyo3::{prelude::*, types::PyDict};

use super::HTTPHandlerBuilder;
use crate::{
    handlers::{HandlerBuilderTrait, socket_builder::BackoffOverrides},
    http_handler::{FemtoHTTPHandler, HTTPMethod},
    macros::{AsPyDict, dict_into_py},
};

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
    #[new]
    fn py_new() -> Self { Self::new() }

    #[pyo3(name = "with_url")]
    #[pyo3(signature = (url))]
    fn py_with_url(mut slf: PyRefMut<'_, Self>, url: String) -> PyRefMut<'_, Self> {
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
        let token_item = config.get_item("token")?;
        let username_item = config.get_item("username")?;
        let password_item = config.get_item("password")?;
        let updated = if let Some(token_value) = token_item {
            if username_item.is_some() || password_item.is_some() {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "with_auth config must not mix 'token' with 'username'/'password'",
                ));
            }
            slf.clone()
                .with_bearer_token(token_value.extract::<String>()?)
        } else {
            let (Some(username_value), Some(password_value)) = (username_item, password_item)
            else {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "with_auth config must specify either 'token' or both 'username' and \
                     'password'",
                ));
            };
            let username_text: String = username_value.extract()?;
            let password_text: String = password_value.extract()?;
            slf.clone().with_basic_auth(username_text, password_text)
        };
        *slf = updated;
        Ok(slf)
    }

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

    #[pyo3(name = "with_bearer_token")]
    #[pyo3(signature = (token))]
    fn py_with_bearer_token(mut slf: PyRefMut<'_, Self>, token: String) -> PyRefMut<'_, Self> {
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
            let extracted_key: String = key.extract()?;
            let extracted_value: String = value.extract()?;
            headers_map.insert(extracted_key, extracted_value);
        }
        let updated = slf.clone().with_headers(headers_map);
        *slf = updated;
        Ok(slf)
    }

    #[pyo3(name = "with_capacity")]
    #[pyo3(signature = (capacity))]
    fn py_with_capacity(mut slf: PyRefMut<'_, Self>, capacity: usize) -> PyRefMut<'_, Self> {
        let updated = slf.clone().with_capacity(capacity);
        *slf = updated;
        slf
    }

    #[pyo3(name = "with_connect_timeout_ms")]
    #[pyo3(signature = (timeout_ms))]
    fn py_with_connect_timeout(mut slf: PyRefMut<'_, Self>, timeout_ms: u64) -> PyRefMut<'_, Self> {
        let updated = slf.clone().with_connect_timeout_ms(timeout_ms);
        *slf = updated;
        slf
    }

    #[pyo3(name = "with_write_timeout_ms")]
    #[pyo3(signature = (timeout_ms))]
    fn py_with_write_timeout(mut slf: PyRefMut<'_, Self>, timeout_ms: u64) -> PyRefMut<'_, Self> {
        let updated = slf.clone().with_write_timeout_ms(timeout_ms);
        *slf = updated;
        slf
    }

    #[pyo3(name = "with_backoff")]
    fn py_with_backoff(
        mut slf: PyRefMut<'_, Self>,
        config: BackoffOverrides,
    ) -> PyRefMut<'_, Self> {
        let updated = slf.clone().with_backoff(config);
        *slf = updated;
        slf
    }

    #[pyo3(name = "with_json_format")]
    fn py_with_json_format(mut slf: PyRefMut<'_, Self>) -> PyRefMut<'_, Self> {
        let updated = slf.clone().with_json_format();
        *slf = updated;
        slf
    }

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

    #[pyo3(name = "as_dict")]
    fn py_as_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let dict = PyDict::new(py);
        self.extend_dict(&dict)?;
        Ok(dict.into())
    }

    #[pyo3(name = "build")]
    fn py_build(&self) -> PyResult<FemtoHTTPHandler> { self.build_inner().map_err(Into::into) }
}

impl AsPyDict for HTTPHandlerBuilder {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let d = PyDict::new(py);
        self.extend_dict(&d)?;
        dict_into_py(d, py)
    }
}

#[cfg(test)]
mod tests {
    //! Tests for the HTTP handler builder Python bindings.

    use pyo3::{
        Python,
        types::{PyAnyMethods, PyDict, PyDictMethods},
    };

    use super::HTTPHandlerBuilder;
    use crate::handlers::HandlerBuilderTrait;

    #[test]
    fn builder_requires_url() {
        Python::attach(|py| {
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
        Python::attach(|py| {
            let builder = HTTPHandlerBuilder::new().with_url("http://localhost:8080/log");
            let d = PyDict::new(py);
            builder
                .extend_dict(&d)
                .expect("dict serialization succeeds");

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
        Python::attach(|py| {
            let builder = HTTPHandlerBuilder::new()
                .with_url("http://localhost:8080/log")
                .with_json_format();
            let d = PyDict::new(py);
            builder
                .extend_dict(&d)
                .expect("dict serialization succeeds");

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
        Python::attach(|py| {
            let builder = HTTPHandlerBuilder::new()
                .with_url("http://localhost:8080/log")
                .with_basic_auth("user", "pass");
            let d = PyDict::new(py);
            builder
                .extend_dict(&d)
                .expect("dict serialization succeeds");

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

    #[test]
    fn with_auth_rejects_mixed_modes() {
        Python::attach(|py| {
            let builder = pyo3::Py::new(py, HTTPHandlerBuilder::new())
                .expect("Py::new should succeed in test");
            let config = PyDict::new(py);
            config
                .set_item("token", "abc-123")
                .expect("token should be set");
            config
                .set_item("username", "user")
                .expect("username should be set");
            config
                .set_item("password", "pass")
                .expect("password should be set");

            let err = HTTPHandlerBuilder::py_with_auth(builder.borrow_mut(py), &config)
                .expect_err("mixed auth config should fail");
            assert!(err.is_instance_of::<pyo3::exceptions::PyValueError>(py));
            assert_eq!(
                err.to_string(),
                "ValueError: with_auth config must not mix 'token' with 'username'/'password'"
            );
        });
    }

    #[test]
    fn with_auth_rejects_incomplete_basic_auth() {
        Python::attach(|py| {
            let builder = pyo3::Py::new(py, HTTPHandlerBuilder::new())
                .expect("Py::new should succeed in test");
            let config = PyDict::new(py);
            config
                .set_item("username", "user")
                .expect("username should be set");

            let err = HTTPHandlerBuilder::py_with_auth(builder.borrow_mut(py), &config)
                .expect_err("incomplete auth config should fail");
            assert!(err.is_instance_of::<pyo3::exceptions::PyValueError>(py));
            assert_eq!(
                err.to_string(),
                "ValueError: with_auth config must specify either 'token' or both 'username' and \
                 'password'"
            );
        });
    }
}
