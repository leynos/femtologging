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

#[cfg(test)]
mod tests {
    use pyo3::PyErr;
    use pyo3::Python;
    use pyo3::types::PyAnyMethods;
    use pyo3::types::PyDict;
    use pyo3::types::PyDictMethods;

    use super::{BackoffOverrides, SocketHandlerBuilder};

    /// Assert all backoff fields on BackoffOverrides match expected values.
    fn assert_backoff_overrides(
        overrides: &BackoffOverrides,
        base_ms: Option<u64>,
        cap_ms: Option<u64>,
        reset_after_ms: Option<u64>,
        deadline_ms: Option<u64>,
    ) {
        assert_eq!(overrides.base_ms, base_ms);
        assert_eq!(overrides.cap_ms, cap_ms);
        assert_eq!(overrides.reset_after_ms, reset_after_ms);
        assert_eq!(overrides.deadline_ms, deadline_ms);
    }

    /// Assert all backoff fields in a PyDict match expected values.
    fn assert_backoff_dict_fields(
        dict: &pyo3::Bound<'_, PyDict>,
        base_ms: Option<u64>,
        cap_ms: Option<u64>,
        reset_after_ms: Option<u64>,
        deadline_ms: Option<u64>,
    ) {
        let get_field = |key: &str| -> Option<u64> {
            dict.get_item(key).unwrap().and_then(|v| v.extract().ok())
        };

        assert_eq!(get_field("backoff_base_ms"), base_ms);
        assert_eq!(get_field("backoff_cap_ms"), cap_ms);
        assert_eq!(get_field("backoff_reset_after_ms"), reset_after_ms);
        assert_eq!(get_field("backoff_deadline_ms"), deadline_ms);
    }

    /// Helper to test BackoffOverrides::py_new error cases.
    /// Takes a closure that populates a PyDict and returns the expected error.
    fn assert_backoff_config_new_error<F>(setup: F, check_error: fn(Python, PyErr))
    where
        F: FnOnce(Python, &pyo3::Bound<'_, PyDict>),
    {
        Python::with_gil(|py| {
            let d = PyDict::new(py);
            setup(py, &d);
            let err = BackoffOverrides::py_new(Some(d)).unwrap_err();
            check_error(py, err);
        });
    }

    #[test]
    fn backoff_config_new_defaults_when_config_is_none() {
        Python::with_gil(|py| {
            let overrides = BackoffOverrides::py_new(None).expect("construct default overrides");
            assert_backoff_overrides(&overrides, None, None, None, None);

            let builder = SocketHandlerBuilder::new().with_backoff(overrides);
            let d = PyDict::new(py);
            builder
                .extend_dict(&d)
                .expect("dict serialisation succeeds");

            assert_backoff_dict_fields(&d, None, None, None, None);
        });
    }

    #[test]
    fn backoff_config_new_accepts_missing_keys() {
        Python::with_gil(|py| {
            let d = PyDict::new(py);
            d.set_item("base_ms", 50_u64).unwrap();

            let overrides =
                BackoffOverrides::py_new(Some(d)).expect("construct overrides with missing keys");
            assert_backoff_overrides(&overrides, Some(50), None, None, None);
        });
    }

    #[test]
    fn backoff_config_new_rejects_unknown_keys() {
        assert_backoff_config_new_error(
            |py, d| {
                d.set_item("base_ms", 50_u64).unwrap();
                d.set_item("typo_ms", 1_u64).unwrap();
            },
            |py, err| {
                assert!(
                    err.is_instance_of::<pyo3::exceptions::PyValueError>(py),
                    "unknown keys should raise ValueError"
                );
            },
        );
    }

    #[test]
    fn backoff_config_new_rejects_invalid_types() {
        assert_backoff_config_new_error(
            |_py, d| {
                d.set_item("base_ms", "not-an-int").unwrap();
            },
            |py, err| {
                assert!(
                    err.is_instance_of::<pyo3::exceptions::PyTypeError>(py),
                    "invalid value types should raise TypeError"
                );
            },
        );
    }

    #[test]
    fn backoff_config_new_treats_explicit_none_as_missing() {
        Python::with_gil(|py| {
            let d = PyDict::new(py);
            d.set_item("base_ms", py.None()).unwrap();
            d.set_item("cap_ms", 500_u64).unwrap();

            let overrides = BackoffOverrides::py_new(Some(d)).expect("construct overrides");
            assert!(overrides.base_ms.is_none());
            assert_eq!(overrides.cap_ms, Some(500));
        });
    }

    #[test]
    fn with_backoff_stores_overrides_on_builder() {
        Python::with_gil(|py| {
            let builder = pyo3::Py::new(py, SocketHandlerBuilder::new()).unwrap();
            let overrides =
                BackoffOverrides::from_options(Some(10), Some(100), Some(200), Some(300));

            let builder_ref = builder.borrow_mut(py);
            let builder_ref =
                SocketHandlerBuilder::py_with_backoff(builder_ref, overrides).expect("apply");
            drop(builder_ref);

            let builder_ref = builder.borrow(py);
            assert_backoff_overrides(
                &builder_ref.backoff,
                Some(10),
                Some(100),
                Some(200),
                Some(300),
            );

            let d = PyDict::new(py);
            builder_ref
                .extend_dict(&d)
                .expect("dict serialisation succeeds");
            assert_eq!(
                d.get_item("backoff_base_ms")
                    .unwrap()
                    .unwrap()
                    .extract::<u64>()
                    .unwrap(),
                10
            );
        });
    }

    #[test]
    fn with_backoff_from_pydict_round_trips_into_builder_dict() {
        Python::with_gil(|py| {
            let d = PyDict::new(py);
            d.set_item("base_ms", 5_u64).unwrap();
            d.set_item("cap_ms", 25_u64).unwrap();

            let overrides = BackoffOverrides::py_new(Some(d)).expect("construct overrides");
            let builder = SocketHandlerBuilder::new().with_backoff(overrides);

            let out = PyDict::new(py);
            builder
                .extend_dict(&out)
                .expect("dict serialisation succeeds");

            assert_backoff_dict_fields(&out, Some(5), Some(25), None, None);
        });
    }
}
