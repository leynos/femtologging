//! Python handler integration helpers for the FemtoLogger module.
//!
//! This module validates Python handler objects and adapts them to the
//! [`FemtoHandlerTrait`] so Rust loggers can drive them safely.

use std::any::Any;

use log::warn;
use pyo3::prelude::*;
use pyo3::{Py, PyAny};

use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::log_record::FemtoLogRecord;

/// Validate that the provided Python object exposes a callable `handle` method.
pub(crate) fn validate_handler(obj: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = obj.py();
    let handle = obj.getattr("handle").map_err(|err| {
        if err.is_instance_of::<pyo3::exceptions::PyAttributeError>(py) {
            pyo3::exceptions::PyTypeError::new_err(
                "handler must implement a callable 'handle' method",
            )
        } else {
            err
        }
    })?;
    if handle.is_callable() {
        Ok(())
    } else {
        let attr_type = handle
            .get_type()
            .name()
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "<unknown>".to_string());
        let handler_repr = obj
            .repr()
            .map(|r| r.to_string())
            .unwrap_or_else(|_| "<unrepresentable>".to_string());
        Err(pyo3::exceptions::PyTypeError::new_err(format!(
            "'handler.handle' is not callable (type: {attr_type}, handler: {handler_repr})",
        )))
    }
}

/// Wrapper allowing Python handler objects to be used by the logger.
pub(crate) struct PyHandler {
    pub(crate) obj: Py<PyAny>,
}

impl FemtoHandlerTrait for PyHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        Python::with_gil(|py| {
            match self.obj.call_method1(
                py,
                "handle",
                (&record.logger, &record.level, &record.message),
            ) {
                Ok(_) => Ok(()),
                Err(err) => {
                    let message = err.to_string();
                    if should_print_py_exceptions() {
                        err.print(py);
                    }
                    warn!("PyHandler: error calling handle: {message}");
                    Err(HandlerError::Message(format!(
                        "python handler raised an exception: {message}"
                    )))
                }
            }
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Flag indicating whether Python exceptions should be printed to stderr.
///
/// Controlled via the `PRINT_PY_EXCEPTIONS` environment variable so operators
/// can opt in to immediate stderr output when debugging Python handlers.
fn should_print_py_exceptions() -> bool {
    std::env::var("PRINT_PY_EXCEPTIONS")
        .ok()
        .map(|value| {
            let lower = value.trim().to_ascii_lowercase();
            matches!(lower.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handler::HandlerError;
    use crate::log_record::FemtoLogRecord;
    use pyo3::types::PyModule;
    use std::ffi::CString;

    fn module_from_code<'py>(py: Python<'py>, source: &str) -> PyResult<Bound<'py, PyModule>> {
        let code = CString::new(source)?;
        let file = CString::new("py_handler_tests.py")?;
        let module_name = CString::new("py_handler_tests")?;
        PyModule::from_code(py, code.as_c_str(), file.as_c_str(), module_name.as_c_str())
    }

    #[test]
    fn validate_handler_accepts_callable() {
        Python::with_gil(|py| -> PyResult<()> {
            let module = module_from_code(
                py,
                "class Handler:
    def handle(self, logger, level, message):
        return message
",
            )?;
            let instance = module.getattr("Handler")?.call0()?;

            validate_handler(&instance)?;
            Ok(())
        })
        .expect("callable handler should validate");
    }

    #[test]
    fn validate_handler_rejects_missing_handle() {
        Python::with_gil(|py| {
            let none = py.None();
            let bound = none.bind(py);
            let err = validate_handler(&bound).expect_err("missing handle must fail");
            assert!(
                err.is_instance_of::<pyo3::exceptions::PyTypeError>(py),
                "error should be a TypeError"
            );
        });
    }

    #[test]
    fn validate_handler_rejects_non_callable_handle() {
        Python::with_gil(|py| {
            let module = module_from_code(
                py,
                "class Handler:
    handle = 123
",
            )
            .expect("module should compile");
            let instance = module
                .getattr("Handler")
                .expect("class should exist")
                .call0()
                .expect("instance construction must succeed");

            let err = validate_handler(&instance).expect_err("non-callable handle must fail");
            assert!(
                err.is_instance_of::<pyo3::exceptions::PyTypeError>(py),
                "error should be a TypeError"
            );
            let message = err.to_string();
            assert!(
                message.contains("not callable"),
                "error message should mention non-callable attribute"
            );
        });
    }

    #[test]
    fn py_handler_invokes_python_handle() {
        std::env::remove_var("PRINT_PY_EXCEPTIONS");
        Python::with_gil(|py| -> PyResult<()> {
            let module = module_from_code(
                py,
                "class Handler:
    def __init__(self):
        self.seen = []
    def handle(self, logger, level, message):
        self.seen.append((logger, level, message))
",
            )?;
            let instance = module.getattr("Handler")?.call0()?;
            let py_obj: Py<PyAny> = instance.unbind();
            let handler = PyHandler {
                obj: py_obj.clone_ref(py),
            };

            handler
                .handle(FemtoLogRecord::new("core", "INFO", "hello"))
                .expect("python handler should succeed");

            let seen: Vec<(String, String, String)> = py_obj.bind(py).getattr("seen")?.extract()?;
            assert_eq!(
                seen,
                vec![("core".to_string(), "INFO".to_string(), "hello".to_string())]
            );
            Ok(())
        })
        .expect("python handler invocation should succeed");
    }

    #[test]
    fn py_handler_returns_handler_error_on_exception() {
        std::env::set_var("PRINT_PY_EXCEPTIONS", "0");
        Python::with_gil(|py| -> PyResult<()> {
            let module = module_from_code(
                py,
                "class Handler:
    def handle(self, logger, level, message):
        raise RuntimeError('fail')
",
            )?;
            let instance = module.getattr("Handler")?.call0()?;
            let py_obj: Py<PyAny> = instance.unbind();
            let handler = PyHandler { obj: py_obj };

            let err = handler
                .handle(FemtoLogRecord::new("core", "INFO", "boom"))
                .expect_err("python exception should map to HandlerError");
            assert_eq!(
                err,
                HandlerError::Message(
                    "python handler raised an exception: RuntimeError: fail".to_string()
                )
            );
            Ok(())
        })
        .expect("python exception should propagate as HandlerError");
        std::env::remove_var("PRINT_PY_EXCEPTIONS");
    }
}
