//! Tests for the socket handler builder Python bindings.

use pyo3::PyErr;
use pyo3::Python;
use pyo3::types::PyAnyMethods;
use pyo3::types::PyDict;
use pyo3::types::PyDictMethods;
use rstest::rstest;

use super::{BackoffOverrides, SocketHandlerBuilder};

/// Assert all backoff fields on `BackoffOverrides` match expected values.
fn assert_backoff_overrides(actual: &BackoffOverrides, expected: &BackoffOverrides) {
    assert_eq!(actual.base_delay, expected.base_delay);
    assert_eq!(actual.max_delay, expected.max_delay);
    assert_eq!(actual.reset_after, expected.reset_after);
    assert_eq!(actual.deadline, expected.deadline);
}

/// Assert all backoff fields in a `PyDict` match expected values.
///
/// Returns an error when a field is missing, fails to extract, or does not
/// match the expected value, so the calling test decides the verdict.
fn assert_backoff_dict_fields(
    dict: &pyo3::Bound<'_, PyDict>,
    expected: &BackoffOverrides,
) -> pyo3::PyResult<()> {
    fn get_field(dict: &pyo3::Bound<'_, PyDict>, key: &str) -> pyo3::PyResult<Option<u64>> {
        dict.get_item(key)?.map(|v| v.extract()).transpose()
    }

    fn expect_field(
        dict: &pyo3::Bound<'_, PyDict>,
        key: &str,
        expected: Option<u64>,
    ) -> pyo3::PyResult<()> {
        let actual = get_field(dict, key)?;
        if actual == expected {
            return Ok(());
        }
        Err(pyo3::exceptions::PyAssertionError::new_err(format!(
            "{key} mismatch: expected {expected:?}, got {actual:?}"
        )))
    }

    expect_field(dict, "backoff_base_ms", expected.base_delay)?;
    expect_field(dict, "backoff_cap_ms", expected.max_delay)?;
    expect_field(dict, "backoff_reset_after_ms", expected.reset_after)?;
    expect_field(dict, "backoff_deadline_ms", expected.deadline)
}

#[test]
fn backoff_config_new_defaults_when_config_is_none() {
    Python::attach(|py| {
        let overrides = BackoffOverrides::py_new(None).expect("construct default overrides");
        let expected = BackoffOverrides::default();
        assert_backoff_overrides(&overrides, &expected);

        let builder = SocketHandlerBuilder::new().with_backoff(overrides);
        let d = PyDict::new(py);
        builder
            .extend_dict(&d)
            .expect("dict serialization succeeds");

        assert_backoff_dict_fields(&d, &expected).expect("backoff dict fields must match");
    });
}

#[test]
fn backoff_config_new_accepts_missing_keys() {
    Python::attach(|py| {
        let d = PyDict::new(py);
        d.set_item("base_ms", 50_u64)
            .expect("set_item should succeed in test");

        let overrides =
            BackoffOverrides::py_new(Some(d)).expect("construct overrides with missing keys");
        let expected = BackoffOverrides::from_options(Some(50), None, None, None);
        assert_backoff_overrides(&overrides, &expected);
    });
}

/// Test helper enum for parameterising validation error scenarios.
///
/// Variants represent different failure modes (unknown keys, invalid types)
/// with methods to set up test cases and verify error types.
#[derive(Clone, Copy)]
enum BackoffErrorKind {
    UnknownKey,
    InvalidType,
}

impl BackoffErrorKind {
    fn setup_dict(self, d: &pyo3::Bound<'_, PyDict>) -> pyo3::PyResult<()> {
        match self {
            Self::UnknownKey => {
                d.set_item("base_ms", 50_u64)?;
                d.set_item("typo_ms", 1_u64)?;
            }
            Self::InvalidType => {
                d.set_item("base_ms", "not-an-int")?;
            }
        }
        Ok(())
    }

    fn check_error(self, py: Python, err: &PyErr) {
        match self {
            Self::UnknownKey => {
                assert!(
                    err.is_instance_of::<pyo3::exceptions::PyValueError>(py),
                    "unknown keys should raise ValueError"
                );
            }
            Self::InvalidType => {
                assert!(
                    err.is_instance_of::<pyo3::exceptions::PyTypeError>(py),
                    "invalid value types should raise TypeError"
                );
            }
        }
    }
}

#[rstest]
#[case::unknown_key(BackoffErrorKind::UnknownKey)]
#[case::invalid_type(BackoffErrorKind::InvalidType)]
fn backoff_config_new_rejects_errors(#[case] kind: BackoffErrorKind) {
    Python::attach(|py| {
        let d = PyDict::new(py);
        kind.setup_dict(&d)
            .expect("set_item should succeed in test");
        let err = BackoffOverrides::py_new(Some(d))
            .expect_err("config with validation errors should fail");
        kind.check_error(py, &err);
    });
}

#[test]
fn backoff_config_new_treats_explicit_none_as_missing() {
    Python::attach(|py| {
        let d = PyDict::new(py);
        d.set_item("base_ms", py.None())
            .expect("set_item should succeed in test");
        d.set_item("cap_ms", 500_u64)
            .expect("set_item should succeed in test");

        let overrides = BackoffOverrides::py_new(Some(d)).expect("construct overrides");
        assert!(overrides.base_delay.is_none());
        assert_eq!(overrides.max_delay, Some(500));
    });
}

#[test]
fn with_backoff_stores_overrides_on_builder() {
    Python::attach(|py| {
        let builder =
            pyo3::Py::new(py, SocketHandlerBuilder::new()).expect("Py::new should succeed in test");
        let expected = BackoffOverrides::from_options(Some(10), Some(100), Some(200), Some(300));

        let mut_ref = builder.borrow_mut(py);
        let applied = SocketHandlerBuilder::py_with_backoff(mut_ref, expected.clone());
        drop(applied);

        let shared = builder.borrow(py);
        assert_backoff_overrides(&shared.backoff, &expected);

        let d = PyDict::new(py);
        shared.extend_dict(&d).expect("dict serialization succeeds");
        assert_backoff_dict_fields(&d, &expected).expect("backoff dict fields must match");
    });
}

#[test]
fn with_backoff_from_pydict_round_trips_into_builder_dict() {
    Python::attach(|py| {
        let d = PyDict::new(py);
        d.set_item("base_ms", 5_u64)
            .expect("set_item should succeed in test");
        d.set_item("cap_ms", 25_u64)
            .expect("set_item should succeed in test");

        let overrides = BackoffOverrides::py_new(Some(d)).expect("construct overrides");
        let builder = SocketHandlerBuilder::new().with_backoff(overrides);

        let out = PyDict::new(py);
        builder
            .extend_dict(&out)
            .expect("dict serialization succeeds");

        let expected = BackoffOverrides::from_options(Some(5), Some(25), None, None);
        assert_backoff_dict_fields(&out, &expected).expect("backoff dict fields must match");
    });
}
