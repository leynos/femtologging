//! Python module registration for feature-gated bindings.
//!
//! This module consolidates all Python-specific class and function
//! registrations, keeping the main `lib.rs` cleaner and reducing
//! scattered `#[cfg(feature = "python")]` annotations.

use pyo3::{Bound, PyResult, prelude::*, wrap_pyfunction};

use crate::{
    ConfigBuilder, FemtoHTTPHandler, FemtoRotatingFileHandler, FemtoSocketHandler,
    FileHandlerBuilder, FilterBuildErrorPy, FormatterBuilder, HTTPHandlerBuilder,
    LevelFilterBuilder, LoggerConfigBuilder, NameFilterBuilder, RotatingFileHandlerBuilder,
    SocketHandlerBuilder, StreamHandlerBuilder,
    handlers::{
        common::PyOverflowPolicy,
        rotating::{
            HandlerOptions, ROTATION_VALIDATION_MSG, clear_rotating_fresh_failure_for_test,
            force_rotating_fresh_failure_for_test,
        },
        socket_builder::BackoffOverrides,
    },
};

/// Register Python-only builders and errors with the module.
///
/// The helper collects registrations in one place, keeping conditional
/// compilation tidy. It is invoked by [`_femtologging_rs`] during
/// initialisation.
pub(crate) fn add_python_bindings(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    // Group type registrations to keep future additions concise.
    for (name, ty) in [
        (
            "StreamHandlerBuilder",
            py.get_type::<StreamHandlerBuilder>(),
        ),
        ("OverflowPolicy", py.get_type::<PyOverflowPolicy>()),
        (
            "SocketHandlerBuilder",
            py.get_type::<SocketHandlerBuilder>(),
        ),
        ("FileHandlerBuilder", py.get_type::<FileHandlerBuilder>()),
        ("HTTPHandlerBuilder", py.get_type::<HTTPHandlerBuilder>()),
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

/// Register Python-only classes with the module.
pub(crate) fn register_python_classes(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<FemtoSocketHandler>()?;
    m.add_class::<FemtoHTTPHandler>()?;
    m.add_class::<FemtoRotatingFileHandler>()?;
    m.add_class::<HandlerOptions>()?;
    m.add_class::<BackoffOverrides>()?;
    m.add("ROTATION_VALIDATION_MSG", ROTATION_VALIDATION_MSG)?;
    Ok(())
}

/// Register Python-only functions with the module.
pub(crate) fn register_python_functions(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(crate::file_config::parse_ini_file, m)?)?;
    m.add_function(wrap_pyfunction!(force_rotating_fresh_failure_for_test, m)?)?;
    m.add_function(wrap_pyfunction!(clear_rotating_fresh_failure_for_test, m)?)?;
    m.add_function(wrap_pyfunction!(crate::frame_filter_py::filter_frames, m)?)?;
    m.add_function(wrap_pyfunction!(
        crate::frame_filter_py::get_logging_infrastructure_patterns,
        m
    )?)?;
    Ok(())
}

/// Register log-compat functions when both python and log-compat features
/// are enabled.
#[cfg(feature = "log-compat")]
pub(crate) fn register_log_compat_functions(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(crate::log_compat::setup_rust_logging, m)?)?;
    m.add_function(wrap_pyfunction!(crate::log_compat::emit_rust_log, m)?)?;
    m.add_function(wrap_pyfunction!(
        crate::log_compat::install_test_global_rust_logger,
        m
    )?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Ensure Python-only bindings register expected types.

    use super::*;
    use pyo3::{
        Python,
        types::{PyModule, PyType},
    };

    #[test]
    fn registers_bindings() {
        // The module should expose builder types and the build error when the
        // `python` feature is enabled.
        Python::with_gil(|py| {
            let module = PyModule::new(py, "test").unwrap();
            add_python_bindings(&module).unwrap();
            for name in [
                "StreamHandlerBuilder",
                "OverflowPolicy",
                "FileHandlerBuilder",
                "HTTPHandlerBuilder",
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
            let module = PyModule::new(py, "_femtologging_rs").unwrap();
            register_python_classes(&module).unwrap();
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
