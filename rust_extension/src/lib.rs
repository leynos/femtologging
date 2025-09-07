//! Python bindings and public re-exports for the femtologging Rust extension.
use pyo3::prelude::*;

mod config;
mod filters;
mod formatter;
mod handler;
mod handlers;
mod level;
mod log_record;
mod logger;
mod macros;
#[cfg(feature = "test-util")]
pub mod manager;
#[cfg(not(feature = "test-util"))]
mod manager;
#[cfg(feature = "test-util")]
pub mod rate_limited_warner;
#[cfg(not(feature = "test-util"))]
mod rate_limited_warner;
mod stream_handler;

pub use config::{ConfigBuilder, FormatterBuilder, LoggerConfigBuilder};
use filters::FilterBuildErrorPy;
pub use filters::{
    FemtoFilter, FilterBuildError, FilterBuilderTrait, LevelFilterBuilder, NameFilterBuilder,
};

pub use formatter::{DefaultFormatter, FemtoFormatter};
pub use handler::{FemtoHandler, FemtoHandlerTrait};
pub use handlers::{
    file::{FemtoFileHandler, HandlerConfig, OverflowPolicy, TestConfig},
    FileHandlerBuilder, HandlerBuilderTrait, HandlerConfigError, HandlerIOError,
    StreamHandlerBuilder,
};
pub use level::FemtoLevel;
pub use log_record::{FemtoLogRecord, RecordMetadata};
pub use logger::{FemtoLogger, QueuedRecord};
use manager::{get_logger as manager_get_logger, reset_manager};
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

#[pymodule]
fn _femtologging_rs(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add_class::<FemtoLogger>()?;
    m.add_class::<FemtoHandler>()?;
    m.add_class::<FemtoStreamHandler>()?;
    m.add_class::<FemtoFileHandler>()?;
    m.add_class::<StreamHandlerBuilder>()?;
    m.add_class::<FileHandlerBuilder>()?;
    m.add_class::<LevelFilterBuilder>()?;
    m.add_class::<NameFilterBuilder>()?;
    m.add("HandlerConfigError", py.get_type::<HandlerConfigError>())?;
    m.add("HandlerIOError", py.get_type::<HandlerIOError>())?;
    m.add("FilterBuildError", py.get_type::<FilterBuildErrorPy>())?;
    m.add_class::<ConfigBuilder>()?;
    m.add_class::<LoggerConfigBuilder>()?;
    m.add_class::<FormatterBuilder>()?;
    m.add_function(wrap_pyfunction!(hello, m)?)?;
    m.add_function(wrap_pyfunction!(get_logger, m)?)?;
    m.add_function(wrap_pyfunction!(reset_manager_py, m)?)?;
    Ok(())
}
