use pyo3::prelude::*;

use crate::log_record::FemtoLogRecord;

#[pyclass]
pub struct FemtoLogger {
    name: String,
}

#[pymethods]
#[allow(non_local_definitions)]
impl FemtoLogger {
    #[new]
    pub fn new(name: String) -> Self {
        Self { name }
    }

    pub fn log(&self, level: &str, message: &str) -> String {
        let record = FemtoLogRecord {
            level: level.to_owned(),
            message: message.to_owned(),
        };
        format!("{}: {:?}", self.name, record)
    }
}
