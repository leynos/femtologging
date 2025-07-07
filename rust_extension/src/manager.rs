use once_cell::sync::Lazy;
use parking_lot::RwLock;
use pyo3::prelude::*;
use std::collections::HashMap;

use crate::logger::FemtoLogger;

#[derive(Default)]
struct Manager {
    loggers: HashMap<String, Py<FemtoLogger>>,
}

static MANAGER: Lazy<RwLock<Manager>> = Lazy::new(|| RwLock::new(Manager::default()));

pub fn get_logger(py: Python<'_>, name: &str) -> PyResult<Py<FemtoLogger>> {
    {
        let mgr = MANAGER.read();
        if let Some(logger) = mgr.loggers.get(name) {
            return Ok(logger.clone_ref(py));
        }
    }

    let parent_name = if let Some((parent, _)) = name.rsplit_once('.') {
        Some(parent.to_string())
    } else if name != "root" {
        Some("root".to_string())
    } else {
        None
    };

    let logger = Py::new(py, FemtoLogger::with_parent(name.to_string(), parent_name))?;
    let mut mgr = MANAGER.write();
    let entry = mgr
        .loggers
        .entry(name.to_string())
        .or_insert_with(|| logger.clone_ref(py));
    Ok(entry.clone_ref(py))
}
