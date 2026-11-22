//! Compile-pass UI test for the builder setters' and `close` PyO3 shapes.
//!
//! The rotating builders' `with_overflow_policy` decodes its argument with
//! `#[pyo3(from_py_with = ...)]`, and their `close` takes a borrowed
//! `Bound<'_, Self>` receiver. Both items live inside the crate's
//! `#[pymethods]` blocks, which an external fixture cannot name, so this
//! fixture pins the two shapes on a local class: a PyO3 upgrade that rejects
//! either form fails here rather than inside the binding modules.

use pyo3::prelude::*;
use pyo3::types::PyAny;

/// Stand-in for the decoded overflow policy.
#[derive(Clone, Copy, Default)]
struct Policy;

/// Stand-in extractor, matching `extract_overflow_policy`'s signature.
fn extract_policy(value: &Bound<'_, PyAny>) -> PyResult<Policy> {
    value.extract::<u8>().map(|_| Policy)
}

#[pyclass]
struct Builder {
    policy: Option<Policy>,
}

#[pymethods]
impl Builder {
    #[new]
    fn py_new() -> Self {
        Self { policy: None }
    }

    /// Mirrors the rotating builder's `from_py_with` setter shape.
    #[pyo3(name = "with_overflow_policy")]
    #[pyo3(signature = (policy))]
    fn py_with_overflow_policy(
        mut slf: PyRefMut<'_, Self>,
        #[pyo3(from_py_with = extract_policy)] policy: Policy,
    ) -> PyResult<PyRefMut<'_, Self>> {
        slf.policy = Some(policy);
        Ok(slf)
    }

    /// Mirrors the rotating handler's borrowed-receiver `close` shape.
    #[pyo3(name = "close")]
    fn py_close(slf: &Bound<'_, Self>, py: Python<'_>) {
        let _ = (slf, py);
    }
}

fn main() {}