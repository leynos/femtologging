use pyo3::prelude::*;

use crate::log_record::FemtoLogRecord;

/// Basic logger used for early experimentation.
#[pyclass]
pub struct FemtoLogger {
    name: String,
}

#[pymethods]
impl FemtoLogger {
    /// Create a new logger with the given name.
    #[new]
    #[pyo3(text_signature = "(name)")]
    pub fn new(name: String) -> Self {
        Self { name }
    }

    /// Format a message at the provided level.
    #[pyo3(text_signature = "(self, level, message)")]
    pub fn log(&self, level: &str, message: &str) -> String {
        let record = FemtoLogRecord {
            level: level.to_owned(),
            message: message.to_owned(),
        };
        format!("{}: {:?}", self.name, record)
    }
}
