use pyo3::prelude::*;

mod formatter;
mod log_record;
mod logger;

pub use formatter::{DefaultFormatter, FemtoFormatter};
pub use log_record::FemtoLogRecord;
pub use logger::FemtoLogger;

#[pyfunction]
fn hello() -> &'static str {
    "hello from Rust"
}

#[pymodule]
fn _femtologging_rs(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<FemtoLogger>()?;
    m.add_function(wrap_pyfunction!(hello, m)?)?;
    Ok(())
}
