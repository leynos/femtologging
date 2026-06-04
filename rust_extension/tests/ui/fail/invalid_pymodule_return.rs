//! Compile-fail UI test for the PyO3 module return contract.
//!
//! This validates that returning a plain integer from a `#[pymodule]` function
//! is rejected by PyO3 instead of silently accepting an invalid module shape.

use pyo3::prelude::*;

#[pyfunction]
fn example() -> PyResult<i32> {
    Ok(1)
}

#[pymodule]
fn bad_module(_py: Python<'_>, module: &Bound<'_, PyModule>) -> i32 {
    module.add_function(wrap_pyfunction!(example, module)?)?;
    0
}

fn main() {}
