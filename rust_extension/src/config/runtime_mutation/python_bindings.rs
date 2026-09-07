//! Python bindings for runtime mutation builders.

use std::collections::BTreeMap;

use super::{CollectionMutation, LoggerMutationBuilder, RuntimeConfigBuilder};
use crate::macros::AsPyDict;
use crate::{FemtoLevel, config::types::HandlerBuilder, filters::FilterBuilder};
use pyo3::IntoPyObjectExt;
use pyo3::prelude::*;
use pyo3::types::PyDict;

fn collection_to_pydict<'py, V: AsPyDict>(
    py: Python<'py>,
    map: &BTreeMap<String, V>,
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    for (key, val) in map {
        dict.set_item(key, val.as_pydict(py)?)?;
    }
    Ok(dict)
}

impl AsPyDict for LoggerMutationBuilder {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let dict = PyDict::new(py);
        if let Some(level) = self.level {
            dict.set_item("level", level.to_string())?;
        }
        if let Some(propagate) = self.propagate {
            dict.set_item("propagate", propagate)?;
        }
        dict.set_item("handlers", self.handlers.as_dict(py)?)?;
        dict.set_item("filters", self.filters.as_dict(py)?)?;
        dict.into_py_any(py)
    }
}

impl AsPyDict for RuntimeConfigBuilder {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let dict = PyDict::new(py);
        if !self.handlers.is_empty() {
            dict.set_item("handlers", collection_to_pydict(py, &self.handlers)?)?;
        }
        if !self.filters.is_empty() {
            dict.set_item("filters", collection_to_pydict(py, &self.filters)?)?;
        }
        if !self.loggers.is_empty() {
            dict.set_item("loggers", collection_to_pydict(py, &self.loggers)?)?;
        }
        if let Some(root) = &self.root_logger {
            dict.set_item("root", root.as_pydict(py)?)?;
        }
        dict.into_py_any(py)
    }
}

#[pymethods]
impl LoggerMutationBuilder {
    #[new]
    fn py_new() -> Self {
        Self::new()
    }

    #[pyo3(name = "with_level")]
    fn py_with_level(mut slf: PyRefMut<'_, Self>, level: FemtoLevel) -> PyRefMut<'_, Self> {
        slf.level = Some(level);
        slf
    }

    #[pyo3(name = "with_propagate")]
    fn py_with_propagate(mut slf: PyRefMut<'_, Self>, propagate: bool) -> PyRefMut<'_, Self> {
        slf.propagate = Some(propagate);
        slf
    }

    #[pyo3(name = "replace_handlers")]
    fn py_replace_handlers(mut slf: PyRefMut<'_, Self>, ids: Vec<String>) -> PyRefMut<'_, Self> {
        slf.set_handlers(CollectionMutation::replace(ids));
        slf
    }

    #[pyo3(name = "append_handlers")]
    fn py_append_handlers(mut slf: PyRefMut<'_, Self>, ids: Vec<String>) -> PyRefMut<'_, Self> {
        slf.set_handlers(CollectionMutation::append(ids));
        slf
    }

    #[pyo3(name = "remove_handlers")]
    fn py_remove_handlers(mut slf: PyRefMut<'_, Self>, ids: Vec<String>) -> PyRefMut<'_, Self> {
        slf.set_handlers(CollectionMutation::remove(ids));
        slf
    }

    #[pyo3(name = "clear_handlers")]
    fn py_clear_handlers(mut slf: PyRefMut<'_, Self>) -> PyRefMut<'_, Self> {
        slf.set_handlers(CollectionMutation::Clear);
        slf
    }

    #[pyo3(name = "replace_filters")]
    fn py_replace_filters(mut slf: PyRefMut<'_, Self>, ids: Vec<String>) -> PyRefMut<'_, Self> {
        slf.set_filters(CollectionMutation::replace(ids));
        slf
    }

    #[pyo3(name = "append_filters")]
    fn py_append_filters(mut slf: PyRefMut<'_, Self>, ids: Vec<String>) -> PyRefMut<'_, Self> {
        slf.set_filters(CollectionMutation::append(ids));
        slf
    }

    #[pyo3(name = "remove_filters")]
    fn py_remove_filters(mut slf: PyRefMut<'_, Self>, ids: Vec<String>) -> PyRefMut<'_, Self> {
        slf.set_filters(CollectionMutation::remove(ids));
        slf
    }

    #[pyo3(name = "clear_filters")]
    fn py_clear_filters(mut slf: PyRefMut<'_, Self>) -> PyRefMut<'_, Self> {
        slf.set_filters(CollectionMutation::Clear);
        slf
    }

    fn as_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.as_pydict(py)
    }
}

#[pymethods]
impl RuntimeConfigBuilder {
    #[new]
    fn py_new() -> Self {
        Self::new()
    }

    #[pyo3(name = "with_handler")]
    fn py_with_handler<'py>(
        mut slf: PyRefMut<'py, Self>,
        id: String,
        builder: &'py Bound<'py, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let handler_builder = builder.extract::<HandlerBuilder>()?;
        slf.handlers.insert(id, handler_builder);
        Ok(slf)
    }

    #[pyo3(name = "with_filter")]
    fn py_with_filter<'py>(
        mut slf: PyRefMut<'py, Self>,
        id: String,
        builder: &'py Bound<'py, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let filter_builder = builder.extract::<FilterBuilder>()?;
        slf.filters.insert(id, filter_builder);
        Ok(slf)
    }

    #[pyo3(name = "with_logger")]
    fn py_with_logger(
        mut slf: PyRefMut<'_, Self>,
        name: String,
        builder: LoggerMutationBuilder,
    ) -> PyRefMut<'_, Self> {
        slf.loggers.insert(name, builder);
        slf
    }

    #[pyo3(name = "with_root_logger")]
    fn py_with_root_logger(
        mut slf: PyRefMut<'_, Self>,
        builder: LoggerMutationBuilder,
    ) -> PyRefMut<'_, Self> {
        slf.root_logger = Some(builder);
        slf
    }

    #[pyo3(name = "apply")]
    fn py_apply(&self) -> PyResult<()> {
        self.apply().map_err(Into::into)
    }

    fn as_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.as_pydict(py)
    }
}
