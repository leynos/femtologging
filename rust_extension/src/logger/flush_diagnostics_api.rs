//! Python accessors for per-logger diagnostic counters.

use pyo3::prelude::*;
use pyo3::types::PyDict;

use super::FemtoLogger;

#[pymethods]
impl FemtoLogger {
    /// Return this logger's fixed-cardinality flush barrier counters.
    ///
    /// The dictionary always contains the same attempt, confirmation, failure
    /// stage and elapsed-time keys. Values are read with relaxed atomics, so
    /// concurrent calls can make the snapshot temporarily inconsistent while
    /// one flush is being recorded. Keys do not include logger names, paths,
    /// messages or payloads.
    ///
    /// The values are suitable for polling by an application metrics adapter;
    /// exporting them to a shared backend remains the caller's responsibility.
    ///
    /// Returns
    /// -------
    /// dict[str, int]
    ///     A stable set of per-logger flush counters and elapsed-time buckets.
    ///
    /// Examples
    /// --------
    /// >>> logger.get_flush_diagnostics()["flush_handlers_attempts"]
    /// 0
    #[pyo3(text_signature = "(self)")]
    pub fn get_flush_diagnostics<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let metrics = self.flush_diagnostics.snapshot().metrics();
        let result = PyDict::new(py);
        for (name, value) in metrics {
            result.set_item(name, value)?;
        }
        Ok(result)
    }

    /// Return how many records were dropped because Python context capture failed.
    #[cfg(feature = "python")]
    #[pyo3(text_signature = "(self)")]
    pub fn get_context_capture_failures(&self) -> u64 {
        self.context_capture_failures
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}
