//! Tests for the HTTP handler builder Python bindings.

use pyo3::Python;
use pyo3::types::{PyAnyMethods, PyDict, PyDictMethods};

use crate::handlers::HandlerBuilderTrait;

use super::HTTPHandlerBuilder;

#[test]
fn builder_requires_url() {
    Python::attach(|py| {
        let builder =
            pyo3::Py::new(py, HTTPHandlerBuilder::new()).expect("Py::new should succeed in test");
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
        let builder =
            pyo3::Py::new(py, HTTPHandlerBuilder::new()).expect("Py::new should succeed in test");
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
        let builder =
            pyo3::Py::new(py, HTTPHandlerBuilder::new()).expect("Py::new should succeed in test");
        let config = PyDict::new(py);
        config
            .set_item("username", "user")
            .expect("username should be set");

        let err = HTTPHandlerBuilder::py_with_auth(builder.borrow_mut(py), &config)
            .expect_err("incomplete auth config should fail");
        assert!(err.is_instance_of::<pyo3::exceptions::PyValueError>(py));
        assert_eq!(
            err.to_string(),
            "ValueError: with_auth config must specify either 'token' or both 'username' and 'password'"
        );
    });
}
