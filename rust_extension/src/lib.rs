//! FemtoLogging Python bindings and public re-exports.
//!
//! This module wires up PyO3 classes and functions exposed to Python and
//! re-exports Rust types used by the Python layer.
use pyo3::prelude::*;

mod config;
mod filters;
mod formatter;
mod handler;
mod handlers;
mod level;
mod log_record;
mod logger;
#[cfg(feature = "python")]
mod macros;
#[cfg(feature = "test-util")]
pub mod manager;
#[cfg(not(feature = "test-util"))]
mod manager;
#[cfg(feature = "python")]
mod python;
#[cfg(feature = "test-util")]
pub mod rate_limited_warner;
#[cfg(not(feature = "test-util"))]
mod rate_limited_warner;
mod stream_handler;

/// Re-export configuration builders for external consumers.
pub use config::{ConfigBuilder, FormatterBuilder, LoggerConfigBuilder};
#[cfg(feature = "python")]
pub use filters::FilterBuildErrorPy;
/// Re-export filter builders and traits.
pub use filters::{
    FemtoFilter, FilterBuildError, FilterBuilderTrait, LevelFilterBuilder, NameFilterBuilder,
};

/// Re-export formatter types.
pub use formatter::{DefaultFormatter, FemtoFormatter};
/// Re-export the base handler trait and wrapper.
pub use handler::{FemtoHandler, FemtoHandlerTrait};
#[cfg(feature = "python")]
pub use handlers::HandlerOptions;
/// Re-export handler builders and errors.
pub use handlers::{
    file::{FemtoFileHandler, HandlerConfig, OverflowPolicy, TestConfig},
    FemtoRotatingFileHandler, FileHandlerBuilder, HandlerBuilderTrait, HandlerConfigError,
    HandlerIOError, RotatingFileHandlerBuilder, StreamHandlerBuilder,
};
/// Re-export logging levels.
pub use level::FemtoLevel;
/// Re-export log record types.
pub use log_record::{FemtoLogRecord, RecordMetadata};
/// Re-export the logger and queued record handle.
pub use logger::{FemtoLogger, QueuedRecord};
use manager::{get_logger as manager_get_logger, reset_manager};
/// Re-export stream handler and config.
pub use stream_handler::{FemtoStreamHandler, HandlerConfig as StreamHandlerConfig};

/// Return a static greeting. Exposed to Python for sanity checks.
///
/// # Returns
///
/// A static string slice containing the greeting.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(crate::hello(), "hello from Rust");
/// ```
#[pyfunction]
fn hello() -> &'static str {
    "hello from Rust"
}

/// Get or create a [`FemtoLogger`] identified by `name`.
///
/// # Parameters
///
/// - `py`: Python GIL token for creating Python objects.
/// - `name`: Logger name; must not be empty, start or end with '.', or contain
///   consecutive dots.
///
/// # Returns
///
/// A reference-counted Python object wrapping the logger. Returns the existing
/// logger when one with the same name has already been created.
///
/// # Errors
///
/// Returns [`PyValueError`](pyo3::exceptions::PyValueError) when the logger
/// name violates the validation rules described above.
///
/// # Examples
///
/// ```rust,ignore
/// # use pyo3::Python;
/// Python::with_gil(|py| {
///     let first = crate::get_logger(py, "example").unwrap();
///     let second = crate::get_logger(py, "example").unwrap();
///     assert!(first.as_ref(py).is(second.as_ref(py)));
/// });
/// ```
#[pyfunction]
fn get_logger(py: Python<'_>, name: &str) -> PyResult<Py<FemtoLogger>> {
    manager_get_logger(py, name)
}

/// Reset the global logging manager state, clearing all registered loggers and
/// handlers.
///
/// Intended for tests; not thread-safe.
///
/// # Examples
///
/// ```rust,ignore
/// # use pyo3::Python;
/// Python::with_gil(|py| {
///     let before = crate::get_logger(py, "example").unwrap();
///     crate::reset_manager_py();
///     let after = crate::get_logger(py, "example").unwrap();
///     assert!(!before.as_ref(py).is(after.as_ref(py)));
/// });
/// ```
#[pyfunction]
fn reset_manager_py() {
    reset_manager();
}

/// Register Python-only builders and errors with the module.
///
/// The helper runs when the `python` feature is enabled and keeps
/// conditional compilation tidy by collecting registrations in one place.
/// It is invoked by [`_femtologging_rs`] during initialisation.
#[cfg(feature = "python")]
fn add_python_bindings(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    // Group type registrations to keep future additions concise.
    for (name, ty) in [
        (
            "StreamHandlerBuilder",
            py.get_type::<StreamHandlerBuilder>(),
        ),
        ("FileHandlerBuilder", py.get_type::<FileHandlerBuilder>()),
        (
            "RotatingFileHandlerBuilder",
            py.get_type::<RotatingFileHandlerBuilder>(),
        ),
        ("LevelFilterBuilder", py.get_type::<LevelFilterBuilder>()),
        ("NameFilterBuilder", py.get_type::<NameFilterBuilder>()),
        ("FilterBuildError", py.get_type::<FilterBuildErrorPy>()),
        ("ConfigBuilder", py.get_type::<ConfigBuilder>()),
        ("LoggerConfigBuilder", py.get_type::<LoggerConfigBuilder>()),
        ("FormatterBuilder", py.get_type::<FormatterBuilder>()),
    ] {
        m.add(name, ty)?;
    }
    Ok(())
}

/// Initialise the `_femtologging_rs` Python extension module.
///
/// # Parameters
///
/// - `m`: The Python module to populate with classes, functions, and
///   constants.
///
/// # Errors
///
/// Returns an error if any class or function registration fails.
///
/// # Examples
///
/// ```rust,ignore
/// # use pyo3::{types::PyModule, Python};
/// Python::with_gil(|py| {
///     let module = PyModule::new(py, "_femtologging_rs").unwrap();
///     crate::_femtologging_rs(&module).unwrap();
///     assert!(module.hasattr("FemtoLogger").unwrap());
/// });
/// ```
#[pymodule]
fn _femtologging_rs(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<FemtoLogger>()?;
    m.add_class::<FemtoHandler>()?;
    m.add_class::<FemtoStreamHandler>()?;
    m.add_class::<FemtoFileHandler>()?;
    #[cfg(feature = "python")]
    m.add_class::<FemtoRotatingFileHandler>()?;
    #[cfg(feature = "python")]
    m.add_class::<HandlerOptions>()?;
    #[cfg(feature = "python")]
    m.add(
        "ROTATION_VALIDATION_MSG",
        handlers::rotating::ROTATION_VALIDATION_MSG,
    )?;
    m.add(
        "HandlerConfigError",
        m.py().get_type::<HandlerConfigError>(),
    )?;
    m.add("HandlerIOError", m.py().get_type::<HandlerIOError>())?;
    #[cfg(feature = "python")]
    // Register builder types and errors that are only compiled when the
    // `python` feature is enabled.
    add_python_bindings(m)?;
    m.add_function(wrap_pyfunction!(hello, m)?)?;
    m.add_function(wrap_pyfunction!(get_logger, m)?)?;
    m.add_function(wrap_pyfunction!(reset_manager_py, m)?)?;
    #[cfg(feature = "python")]
    m.add_function(wrap_pyfunction!(
        handlers::rotating::force_rotating_fresh_failure_for_test,
        m
    )?)?;
    #[cfg(feature = "python")]
    m.add_function(wrap_pyfunction!(
        handlers::rotating::clear_rotating_fresh_failure_for_test,
        m
    )?)?;

    Ok(())
}

#[cfg(all(test, feature = "python"))]
mod tests {
    //! Ensure Python-only bindings register expected types.

    use super::*;
    use crate::handlers::rotating::ROTATION_VALIDATION_MSG;
    use pyo3::{
        types::{PyModule, PyType},
        Python,
    };

    #[test]
    fn registers_bindings() {
        // The module should expose builder types and the build error when the
        // `python` feature is enabled.
        Python::with_gil(|py| {
            let module = PyModule::new(py, "test").unwrap().bind(py);
            add_python_bindings(&module).unwrap();
            for name in [
                "StreamHandlerBuilder",
                "FileHandlerBuilder",
                "RotatingFileHandlerBuilder",
                "LevelFilterBuilder",
                "NameFilterBuilder",
                "FilterBuildError",
                "ConfigBuilder",
                "LoggerConfigBuilder",
                "FormatterBuilder",
            ] {
                // Ensure each registration exists and is a Python type.
                let attr = module.getattr(name).unwrap();
                attr.downcast::<PyType>().unwrap();
            }
        });
    }

    #[test]
    fn module_registers_rotating_classes() {
        Python::with_gil(|py| {
            let module = PyModule::new(py, "_femtologging_rs").unwrap().bind(py);
            super::_femtologging_rs(&module).unwrap();
            for name in ["FemtoRotatingFileHandler", "HandlerOptions"] {
                let attr = module.getattr(name).unwrap();
                attr.downcast::<PyType>().unwrap();
            }
            let message = module.getattr("ROTATION_VALIDATION_MSG").unwrap();
            let value: &str = message.extract().unwrap();
            assert_eq!(value, ROTATION_VALIDATION_MSG);
        });
    }
}
