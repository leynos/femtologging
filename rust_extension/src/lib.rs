#![allow(non_local_definitions)]

use pyo3::prelude::*;

mod log_record;
mod logger;

use logger::FemtoLogger;

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
