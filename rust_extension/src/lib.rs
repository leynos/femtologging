//! FemtoLogging Python bindings and public re-exports.
//!
//! This module wires up PyO3 classes and functions exposed to Python and
//! re-exports Rust types used by the Python layer.
use pyo3::prelude::*;

// Core modules (always compiled)
mod config;
pub mod exception_schema;
mod filters;
mod formatter;
pub mod frame_filter;
mod handler;
mod handlers;
mod http_handler;
mod level;
mod log_record;
mod logger;
mod socket_handler;
mod stream_handler;

// Feature-gated manager visibility
#[cfg(feature = "test-util")]
pub mod manager;
#[cfg(not(feature = "test-util"))]
mod manager;
#[cfg(feature = "test-util")]
pub mod rate_limited_warner;
#[cfg(not(feature = "test-util"))]
mod rate_limited_warner;

// Logging macros (always compiled for Rust callers)
mod logging_macros;

// Python-only modules
#[cfg(feature = "python")]
mod convenience_functions;
#[cfg(feature = "python")]
mod file_config;
#[cfg(feature = "python")]
mod frame_filter_py;
#[cfg(feature = "python")]
mod macros;
#[cfg(feature = "python")]
mod python;
#[cfg(feature = "python")]
mod python_module;
#[cfg(feature = "python")]
pub(crate) mod traceback_capture;
#[cfg(feature = "python")]
pub(crate) mod traceback_frames;

// Feature-gated log-compat module
#[cfg(all(feature = "python", feature = "log-compat"))]
mod log_compat;

// Test modules
#[cfg(test)]
mod test_utils;
#[cfg(all(test, feature = "python"))]
mod traceback_capture_graceful_degradation_tests;
#[cfg(all(test, feature = "python"))]
mod traceback_capture_tests;
#[cfg(all(test, feature = "python"))]
mod traceback_frames_graceful_degradation_tests;
#[cfg(all(test, feature = "python"))]
mod traceback_frames_tests;

// Re-exports: configuration builders
pub use config::{ConfigBuilder, FormatterBuilder, LoggerConfigBuilder};

// Re-exports: filter types (FilterBuildErrorPy is Python-only)
#[cfg(feature = "python")]
pub use filters::FilterBuildErrorPy;
pub use filters::{
    FemtoFilter, FilterBuildError, FilterBuilderTrait, LevelFilterBuilder, NameFilterBuilder,
};

/// Re-export exception schema types.
pub use exception_schema::{
    EXCEPTION_SCHEMA_VERSION, ExceptionPayload, MIN_EXCEPTION_SCHEMA_VERSION, SchemaVersionError,
    SchemaVersioned, StackFrame, StackTracePayload, validate_schema_version,
};
/// Re-export formatter types.
pub use formatter::{DefaultFormatter, ExceptionFormat, FemtoFormatter};
/// Re-export the base handler trait and wrapper.
pub use handler::{FemtoHandler, FemtoHandlerTrait, HandlerError};
#[cfg(feature = "python")]
pub use handlers::HandlerOptions;
/// Re-export handler builders and errors.
pub use handlers::{
    FemtoRotatingFileHandler, FileHandlerBuilder, HTTPHandlerBuilder, HandlerBuilderTrait,
    HandlerConfigError, HandlerIOError, RotatingFileHandlerBuilder, SocketHandlerBuilder,
    StreamHandlerBuilder,
    file::{FemtoFileHandler, HandlerConfig, OverflowPolicy, TestConfig},
};
/// Re-export HTTP handler types.
pub use http_handler::{
    AuthConfig, FemtoHTTPHandler, HTTPHandlerConfig, HTTPMethod, SerializationFormat,
};
/// Re-export logging levels.
pub use level::FemtoLevel;
/// Re-export log record types.
pub use log_record::{FemtoLogRecord, RecordMetadata};
/// Re-export the logger and queued record handle.
pub use logger::{FemtoLogger, QueuedRecord};
use manager::{get_logger as manager_get_logger, reset_manager};
pub use socket_handler::{
    BackoffPolicy, FemtoSocketHandler, SocketHandlerConfig, SocketTransport, TcpTransport,
    TlsOptions, UnixTransport,
};
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

#[allow(
    clippy::too_many_arguments,
    reason = "PyO3 macro-generated wrappers expand Python-call signatures"
)]
mod py_api {
    //! Python-facing helper functions that bridge to the Rust manager.

    use super::*;

    /// Get or create a [`FemtoLogger`] identified by `name`.
    ///
    /// # Parameters
    ///
    /// - `py`: Python GIL token for creating Python objects.
    /// - `name`: Logger name; must not be empty, start or end with '.', or
    ///   contain consecutive dots.
    ///
    /// # Returns
    ///
    /// A reference-counted Python object wrapping the logger. Returns the
    /// existing logger when one with the same name has already been created.
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
    /// Python::attach(|py| {
    ///     let first = crate::get_logger(py, "example").unwrap();
    ///     let second = crate::get_logger(py, "example").unwrap();
    ///     assert!(first.as_ref(py).is(second.as_ref(py)));
    /// });
    /// ```
    #[allow(
        clippy::too_many_arguments,
        reason = "PyO3 expands function wrappers with Python-call compatibility arguments"
    )]
    #[pyfunction]
    pub(crate) fn get_logger(py: Python<'_>, name: &str) -> PyResult<Py<FemtoLogger>> {
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
    /// Python::attach(|py| {
    ///     let before = crate::get_logger(py, "example").unwrap();
    ///     crate::reset_manager_py();
    ///     let after = crate::get_logger(py, "example").unwrap();
    ///     assert!(!before.as_ref(py).is(after.as_ref(py)));
    /// });
    /// ```
    #[pyfunction]
    pub(crate) fn reset_manager_py() {
        reset_manager();
    }
}

use py_api::{get_logger, reset_manager_py};

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
/// Python::attach(|py| {
///     let module = PyModule::new(py, "_femtologging_rs").unwrap();
///     crate::_femtologging_rs(&module).unwrap();
///     assert!(module.hasattr("FemtoLogger").unwrap());
/// });
/// ```
#[pymodule]
fn _femtologging_rs(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Core classes (always registered)
    m.add_class::<FemtoLogger>()?;
    m.add_class::<FemtoHandler>()?;
    m.add_class::<FemtoStreamHandler>()?;
    m.add_class::<FemtoFileHandler>()?;
    m.add(
        "HandlerConfigError",
        m.py().get_type::<HandlerConfigError>(),
    )?;
    m.add("HandlerIOError", m.py().get_type::<HandlerIOError>())?;
    m.add("EXCEPTION_SCHEMA_VERSION", EXCEPTION_SCHEMA_VERSION)?;

    // Core functions (always registered)
    m.add_function(wrap_pyfunction!(hello, m)?)?;
    m.add_function(wrap_pyfunction!(get_logger, m)?)?;
    m.add_function(wrap_pyfunction!(reset_manager_py, m)?)?;

    // Python-only classes, builders, and functions
    #[cfg(feature = "python")]
    {
        python_module::register_python_classes(m)?;
        python_module::add_python_bindings(m)?;
        python_module::register_python_functions(m)?;
    }

    // Log-compat functions (requires both python and log-compat features)
    #[cfg(all(feature = "python", feature = "log-compat"))]
    python_module::register_log_compat_functions(m)?;

    Ok(())
}

// Tests are now in python_module.rs when the python feature is enabled.
