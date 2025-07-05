use pyo3::prelude::*;

mod file_handler;
mod formatter;
mod handler;
mod level;
mod log_record;
mod logger;
mod stream_handler;

pub use file_handler::FemtoFileHandler;
pub use formatter::{DefaultFormatter, FemtoFormatter};
pub use handler::{FemtoHandler, FemtoHandlerTrait};
pub use level::FemtoLevel;
pub use log_record::{FemtoLogRecord, RecordMetadata};
pub use logger::FemtoLogger;
pub use stream_handler::FemtoStreamHandler;

#[pyfunction]
fn hello() -> &'static str {
    "hello from Rust"
}

#[allow(deprecated)]
#[pymodule]
fn _femtologging_rs(_py: Python<'_>, m: &PyModule) -> PyResult<()> {
    m.add_class::<FemtoLogger>()?;
    m.add_class::<FemtoLevel>()?;
    m.add_class::<FemtoHandler>()?;
    m.add_class::<FemtoStreamHandler>()?;
    m.add_class::<FemtoFileHandler>()?;
    m.add_function(wrap_pyfunction!(hello, m)?)?;
    Ok(())
}
