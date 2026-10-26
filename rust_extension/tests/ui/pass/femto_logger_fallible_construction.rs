//! Compile-pass coverage for the public fallible logger constructors.

use _femtologging_rs::{DispatcherError, FemtoLogger};
use pyo3::PyResult;

fn python_constructor(name: String) -> PyResult<FemtoLogger> {
    let logger = FemtoLogger::new(name)?;
    Ok(logger)
}

fn rust_constructor(name: String) -> Result<FemtoLogger, DispatcherError> {
    let logger = FemtoLogger::with_parent(name, None)?;
    Ok(logger)
}

fn main() {
    let _: fn(String) -> PyResult<FemtoLogger> = python_constructor;
    let _: fn(String) -> Result<FemtoLogger, DispatcherError> = rust_constructor;
}
