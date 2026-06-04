//! Compile-pass UI test for PyO3 signatures with default arguments.
//!
//! This validates the explicit signature form used by femtologging's Python
//! bindings when optional keyword arguments need stable Python call metadata.

use pyo3::prelude::*;

#[pyfunction]
#[pyo3(signature = (message, /, *, name=None))]
fn example_log(message: &str, name: Option<&str>) -> PyResult<usize> {
    Ok(message.len() + name.unwrap_or_default().len())
}

fn main() {}
