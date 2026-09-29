//! Compile-pass UI test for PyO3 signatures with default arguments.
//!
//! This validates the explicit signature form used by femtologging's Python
//! bindings when optional keyword arguments need stable Python call metadata.

use pyo3::prelude::*;
use pyo3::types::PyAny;

#[pyfunction]
#[pyo3(signature = (message, /, *, name=None, extra=None))]
fn example_log(
    message: &str,
    name: Option<&str>,
    extra: Option<&Bound<'_, PyAny>>,
) -> PyResult<usize> {
    let extra_present = if extra.is_some() { 1 } else { 0 };
    Ok(message.len() + name.unwrap_or_default().len() + extra_present)
}

fn main() {}
