//! Compile-pass UI test for the `from_py_with` argument attribute.
//!
//! The file builder's overflow-policy setter decodes its Python wrapper with a
//! custom extractor. This fixture pins the attribute form itself, so a PyO3
//! upgrade that rejects it fails here rather than inside the binding module.

use pyo3::prelude::*;
use pyo3::types::PyAny;

fn parse_positive(value: &Bound<'_, PyAny>) -> PyResult<u64> {
    value.extract()
}

#[pyfunction]
fn bump(#[pyo3(from_py_with = parse_positive)] by: u64) -> u64 {
    by + 1
}

fn main() {}