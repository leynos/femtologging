//! Python callback filter support for stdlib-compatible filtering.
//!
//! This module adapts Python callables and `logging.Filter`-style objects to
//! the Rust filter pipeline. Callback execution happens on the producer thread,
//! and accepted enrichment fields are copied into Rust-owned record metadata
//! before asynchronous dispatch.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use log::warn;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::formatter::python::record_to_dict;
use crate::log_record::FemtoLogRecord;
use crate::macros::{AsPyDict, dict_into_py};
use crate::python::fq_py_type;

use super::python_callback_validation::is_reserved_enrichment_key;
use super::{FemtoFilter, FilterBuildError, FilterBuilderTrait, FilterContext, FilterDecision};

#[path = "python_callback_enrichment.rs"]
mod enrichment;
use enrichment::extract_enrichment;

type SerializedEnrichment = BTreeMap<String, String>;
type TypedEnrichment = BTreeMap<String, Py<PyAny>>;

/// A filter backed by a Python callable or `logging.Filter`-style object.
#[derive(Clone, Debug)]
pub struct PythonCallbackFilter {
    callback: Arc<Mutex<Py<PyAny>>>,
    description: String,
}

impl PythonCallbackFilter {
    /// Build a new Python callback filter from a validated Python object.
    pub fn new(callback: Py<PyAny>, description: String) -> Self {
        Self {
            callback: Arc::new(Mutex::new(callback)),
            description,
        }
    }

    fn decision_with_context(
        &self,
        record: &FemtoLogRecord,
        context: &mut FilterContext,
    ) -> Result<FilterDecision, PyErr> {
        Python::attach(|py| {
            let record_view = get_or_create_filter_record(py, record, context)?;
            let before = snapshot_record_attrs(&record_view)?;
            let outcome = self.evaluate_record_view(py, &record_view, &before);

            if outcome.is_err() {
                context.python_record_view = None;
                warn_if_record_restore_fails(&record_view, &before);
            }
            outcome
        })
    }

    fn evaluate_record_view<'py>(
        &self,
        py: Python<'py>,
        record_view: &Bound<'py, PyAny>,
        before: &BTreeMap<String, Py<PyAny>>,
    ) -> PyResult<FilterDecision> {
        let result = self.invoke(py, record_view)?;
        let accepted = result.is_truthy()?;
        let (enrichment, typed_enrichment) =
            extract_enrichment(py, record_view, before, &self.description)?;
        restore_record_attrs(record_view, before)?;
        if !accepted {
            return Ok(FilterDecision::accept(false));
        }
        apply_enrichment_to_record_view(record_view, &enrichment, &typed_enrichment)?;
        Ok(FilterDecision {
            accepted: true,
            enrichment,
        })
    }

    fn invoke<'py>(
        &self,
        py: Python<'py>,
        record_view: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let callback_object = self
            .callback
            .lock()
            .map_err(|_| {
                pyo3::exceptions::PyRuntimeError::new_err("python callback filter mutex poisoned")
            })?
            .clone_ref(py);
        let callback = callback_object.bind(py);
        if callback.is_callable() {
            callback.call1((record_view,))
        } else {
            callback.getattr("filter")?.call1((record_view,))
        }
    }

    pub(crate) fn description(&self) -> &str {
        &self.description
    }
}

fn warn_if_record_restore_fails(
    record_view: &Bound<'_, PyAny>,
    before: &BTreeMap<String, Py<PyAny>>,
) {
    if let Err(restore_err) = restore_record_attrs(record_view, before) {
        warn!("failed to restore Python filter record after an error: {restore_err}");
    }
}

impl FemtoFilter for PythonCallbackFilter {
    fn decision(&self, record: &mut FemtoLogRecord, context: &mut FilterContext) -> FilterDecision {
        match self.decision_with_context(record, context) {
            Ok(decision) => decision,
            Err(err) => {
                warn!(
                    "Python filter callback '{}' raised an exception; record dropped: {}",
                    self.description, err
                );
                FilterDecision::accept(false)
            }
        }
    }
}

/// Builder for a Python callback filter.
#[pyclass(from_py_object)]
#[derive(Clone, Debug)]
pub struct PythonCallbackFilterBuilder {
    filter: PythonCallbackFilter,
}

impl PythonCallbackFilterBuilder {
    /// Create a new builder from a validated callback object.
    ///
    /// # Errors
    ///
    /// Returns a Python error when `obj` is neither callable nor an object with
    /// a callable `filter` method.
    pub fn from_callback_obj(obj: Bound<'_, PyAny>) -> PyResult<Self> {
        validate_filter_target(&obj)?;
        let description = fq_py_type(&obj);
        Ok(Self {
            filter: PythonCallbackFilter::new(obj.unbind(), description),
        })
    }
}

impl FilterBuilderTrait for PythonCallbackFilterBuilder {
    type Filter = PythonCallbackFilter;

    fn build_inner(&self) -> Result<Self::Filter, FilterBuildError> {
        Ok(self.filter.clone())
    }
}

impl AsPyDict for PythonCallbackFilterBuilder {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let dict = PyDict::new(py);
        dict.set_item("callable_type", self.filter.description())?;
        dict_into_py(dict, py)
    }
}

#[pymethods]
impl PythonCallbackFilterBuilder {
    #[new]
    #[pyo3(text_signature = "(callback, /)")]
    fn py_new(callback: Bound<'_, PyAny>) -> PyResult<Self> {
        Self::from_callback_obj(callback)
    }

    fn as_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.as_pydict(py)
    }
}

pub(crate) fn validate_filter_target(obj: &Bound<'_, PyAny>) -> PyResult<()> {
    if obj.is_callable() {
        return Ok(());
    }

    match obj.getattr("filter") {
        Ok(filter_method) if filter_method.is_callable() => Ok(()),
        Ok(_) => Err(pyo3::exceptions::PyTypeError::new_err(format!(
            "python callback filter must be callable or expose a callable 'filter' method (got {})",
            fq_py_type(obj),
        ))),
        Err(err) if err.is_instance_of::<pyo3::exceptions::PyAttributeError>(obj.py()) => {
            Err(pyo3::exceptions::PyTypeError::new_err(format!(
                "python callback filter must be callable or expose a callable 'filter' method (got {})",
                fq_py_type(obj),
            )))
        }
        Err(err) => Err(err),
    }
}

fn get_or_create_filter_record<'py>(
    py: Python<'py>,
    record: &FemtoLogRecord,
    context: &mut FilterContext,
) -> PyResult<Bound<'py, PyAny>> {
    if context.python_record_view.is_none() {
        context.python_record_view = Some(create_filter_record(py, record)?.unbind());
    }
    Ok(context
        .python_record_view
        .as_ref()
        .ok_or_else(|| {
            pyo3::exceptions::PyRuntimeError::new_err("python record view not initialized")
        })?
        .bind(py)
        .clone())
}

fn create_filter_record<'py>(
    py: Python<'py>,
    record: &FemtoLogRecord,
) -> PyResult<Bound<'py, PyAny>> {
    let record_dict_obj = record_to_dict(py, record)?;
    let record_dict = record_dict_obj.bind(py).cast::<PyDict>()?;
    let metadata = record_dict
        .get_item("metadata")?
        .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("record dict missing metadata"))?;
    let metadata_dict = metadata.cast::<PyDict>()?;
    let extra_binding = metadata_dict.get_item("key_values")?.ok_or_else(|| {
        pyo3::exceptions::PyRuntimeError::new_err("record dict missing key_values")
    })?;
    let extra = extra_binding.cast::<PyDict>()?;

    let log_record_payload = PyDict::new(py);
    log_record_payload.set_item("name", record.logger())?;
    log_record_payload.set_item("msg", record.message())?;
    log_record_payload.set_item("levelname", record.level_str())?;
    log_record_payload.set_item("levelno", u8::from(record.level()))?;
    log_record_payload.set_item("pathname", &record.metadata().filename)?;
    log_record_payload.set_item("filename", &record.metadata().filename)?;
    log_record_payload.set_item("module", &record.metadata().module_path)?;
    log_record_payload.set_item("lineno", record.metadata().line_number)?;
    log_record_payload.set_item("logger", record.logger())?;
    log_record_payload.set_item("level", record.level_str())?;
    log_record_payload.set_item("message", record.message())?;
    log_record_payload.set_item("metadata", metadata)?;
    log_record_payload.set_item("args", PyList::empty(py))?;
    if let Some(exc_info) = record_dict.get_item("exc_info")? {
        log_record_payload.set_item("exc_info", exc_info)?;
    }
    if let Some(stack_info) = record_dict.get_item("stack_info")? {
        log_record_payload.set_item("stack_info", stack_info)?;
    }
    for (key, value) in extra.iter() {
        let key_string = key.extract::<String>()?;
        if is_reserved_enrichment_key(&key_string) {
            continue;
        }
        log_record_payload.set_item(key_string, value)?;
    }

    let logging = py.import("logging")?;
    logging.call_method1("makeLogRecord", (log_record_payload,))
}

fn snapshot_record_attrs(record_view: &Bound<'_, PyAny>) -> PyResult<BTreeMap<String, Py<PyAny>>> {
    let py = record_view.py();
    let copy = py.import("copy")?;
    let binding = record_view.getattr("__dict__")?;
    let dict = binding.cast::<PyDict>()?;
    let mut attrs = BTreeMap::new();
    for (key, value) in dict.iter() {
        let copied = copy.call_method1("deepcopy", (value,))?;
        attrs.insert(key.extract::<String>()?, copied.unbind());
    }
    Ok(attrs)
}

fn restore_record_attrs(
    record_view: &Bound<'_, PyAny>,
    before: &BTreeMap<String, Py<PyAny>>,
) -> PyResult<()> {
    let py = record_view.py();
    let binding = record_view.getattr("__dict__")?;
    let dict = binding.cast::<PyDict>()?;
    let keys = dict.keys();
    for key in keys.iter() {
        let key_string = key.extract::<String>()?;
        if !before.contains_key(&key_string) {
            dict.del_item(&key_string)?;
        }
    }
    for (key, value) in before {
        dict.set_item(key, value.bind(py))?;
    }
    Ok(())
}

fn apply_enrichment_to_record_view(
    record_view: &Bound<'_, PyAny>,
    enrichment: &SerializedEnrichment,
    typed_enrichment: &TypedEnrichment,
) -> PyResult<()> {
    let py = record_view.py();
    let binding = record_view.getattr("__dict__")?;
    let dict = binding.cast::<PyDict>()?;
    for (key, value) in typed_enrichment {
        dict.set_item(key, value.bind(py))?;
    }
    if let Some(metadata) = dict.get_item("metadata")? {
        let metadata_dict = metadata.cast::<PyDict>()?;
        if let Some(key_values) = metadata_dict.get_item("key_values")? {
            let key_values_dict = key_values.cast::<PyDict>()?;
            for (key, value) in typed_enrichment {
                key_values_dict.set_item(key, value.bind(py))?;
            }
        }
    }
    debug_assert_eq!(
        enrichment.len(),
        typed_enrichment.len(),
        "validated enrichment maps must have matching lengths"
    );
    Ok(())
}
