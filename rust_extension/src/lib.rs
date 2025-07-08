use pyo3::prelude::*;

mod file_handler;
mod formatter;
mod handler;
mod level;
mod log_record;
mod logger;
mod manager;
mod stream_handler;

pub use file_handler::{FemtoFileHandler, HandlerConfig, OverflowPolicy, TestConfig};
pub use formatter::{DefaultFormatter, FemtoFormatter};
pub use handler::{FemtoHandler, FemtoHandlerTrait};
pub use level::FemtoLevel;
pub use log_record::{FemtoLogRecord, RecordMetadata};
pub use logger::FemtoLogger;
use manager::{get_logger as manager_get_logger, reset_manager};
pub use stream_handler::FemtoStreamHandler;

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

#[allow(deprecated)]
#[pymodule]
fn _femtologging_rs(_py: Python<'_>, m: &PyModule) -> PyResult<()> {
    m.add_class::<FemtoLogger>()?;
    m.add_class::<FemtoHandler>()?;
    m.add_class::<FemtoStreamHandler>()?;
    m.add_class::<FemtoFileHandler>()?;
    m.add_function(wrap_pyfunction!(hello, m)?)?;
    m.add_function(wrap_pyfunction!(get_logger, m)?)?;
    m.add_function(wrap_pyfunction!(reset_manager_py, m)?)?;
    Ok(())
}
