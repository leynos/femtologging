//! Compile-pass UI test for a well-formed `#[pymodule]` definition.
//!
//! This validates that a module function returning `PyResult<()>` and
//! registering a `#[pyfunction]` compiles with the project's PyO3 version.

use pyo3::prelude::*;

#[pyfunction]
fn is_available(_py: Python<'_>) -> PyResult<bool> {
    Ok(true)
}

#[pymodule]
fn compile_pass_module(py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(is_available, module)?)?;
    module.add("__doc__", "femtologging Rust extension.")?;
    module.add("__package__", "femtologging")?;
    module.add("__loader__", py.None())?;
    Ok(())
}

fn main() {}
