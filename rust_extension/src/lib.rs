//! Python bindings and public re-exports for the femtologging Rust extension.
//!
//! The [`_femtologging_rs`] module exposes core logging types to Python.
//! When the `python` feature is enabled, [`add_python_bindings`] registers
//! builders and errors that are otherwise conditionally compiled. The crate
//! re-exports these types so they remain usable from Rust.
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
/// Re-export handler builders and errors.
pub use handlers::{
    file::{FemtoFileHandler, HandlerConfig, OverflowPolicy, TestConfig},
    FileHandlerBuilder, HandlerBuilderTrait, HandlerConfigError, HandlerIOError,
    StreamHandlerBuilder,
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

#[pyfunction]
fn hello() -> &'static str {
    "hello from Rust"
}

#[pyfunction]
fn get_logger(py: Python<'_>, name: &str) -> PyResult<Py<FemtoLogger>> {
    manager_get_logger(py, name)
}

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

#[pymodule]
fn _femtologging_rs(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<FemtoLogger>()?;
    m.add_class::<FemtoHandler>()?;
    m.add_class::<FemtoStreamHandler>()?;
    m.add_class::<FemtoFileHandler>()?;
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
    Ok(())
}
