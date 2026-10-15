//! Producer-side provider for Python execution-context snapshots.
//!
//! Keeping Python's `contextvars` API behind this small interface lets the
//! logger pair a testable snapshot provider with each record's handler list.

use pyo3::prelude::*;
use pyo3::types::PyAnyMethods;

/// Supplies the Python context attached to a queued record.
pub(super) trait ContextSnapshotProvider: Send + Sync {
    /// Capture the current Python context on the producer thread.
    fn capture(&self) -> PyResult<Py<PyAny>>;
}

/// Default provider backed by Python's `contextvars.copy_context()`.
pub(super) struct ContextVarsSnapshotProvider;

impl ContextSnapshotProvider for ContextVarsSnapshotProvider {
    fn capture(&self) -> PyResult<Py<PyAny>> {
        Python::attach(|py| {
            py.import("contextvars")
                .and_then(|module| module.call_method0("copy_context"))
                .map(Bound::unbind)
        })
    }
}
