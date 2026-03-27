//! Python callback filter support for stdlib-compatible filtering.
//!
//! This module adapts Python callables and `logging.Filter`-style objects to
//! the Rust filter pipeline. Callback execution happens on the producer thread,
//! and accepted enrichment fields are copied into Rust-owned record metadata
//! before asynchronous dispatch.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use log::warn;
use pyo3::basic::CompareOp;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::formatter::python::record_to_dict;
use crate::log_record::FemtoLogRecord;
use crate::macros::{AsPyDict, dict_into_py};
use crate::python::fq_py_type;

use super::python_callback_validation::{
    extract_supported_value, validate_enrichment_key, validate_enrichment_total,
    validate_enrichment_value,
};
use super::{FemtoFilter, FilterBuildError, FilterBuilderTrait};

#[derive(Debug)]
pub(crate) struct FilterDecision {
    pub(crate) accepted: bool,
    pub(crate) enrichment: BTreeMap<String, String>,
}

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

    pub(crate) fn filter_with_enrichment(
        &self,
        record: &FemtoLogRecord,
    ) -> Result<FilterDecision, PyErr> {
        Python::attach(|py| {
            let record_view = create_filter_record(py, record)?;
            let before = snapshot_record_attrs(&record_view)?;
            let result = self.invoke(py, &record_view)?;
            let accepted = result.is_truthy()?;
            let enrichment = extract_enrichment(py, &record_view, &before, &self.description)?;
            Ok(FilterDecision {
                accepted,
                enrichment,
            })
        })
    }

    fn invoke<'py>(
        &self,
        py: Python<'py>,
        record_view: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let callback = self
            .callback
            .lock()
            .expect("python callback filter mutex should not be poisoned")
            .clone_ref(py);
        let callback = callback.bind(py);
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

impl FemtoFilter for PythonCallbackFilter {
    fn should_log(&self, record: &FemtoLogRecord) -> bool {
        match self.filter_with_enrichment(record) {
            Ok(decision) => decision.accepted,
            Err(err) => {
                warn!(
                    "Python filter callback '{}' raised an exception; record dropped: {}",
                    self.description, err
                );
                false
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

    let filter_method = obj.getattr("filter")?;
    if filter_method.is_callable() {
        return Ok(());
    }

    Err(pyo3::exceptions::PyTypeError::new_err(format!(
        "python callback filter must be callable or expose a callable 'filter' method (got {})",
        fq_py_type(obj),
    )))
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
    let metadata = metadata.cast::<PyDict>()?;
    let extra_binding = metadata.get_item("key_values")?.ok_or_else(|| {
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
    for (key, value) in extra.iter() {
        log_record_payload.set_item(key, value)?;
    }

    let logging = py.import("logging")?;
    logging.call_method1("makeLogRecord", (log_record_payload,))
}

fn snapshot_record_attrs(record_view: &Bound<'_, PyAny>) -> PyResult<BTreeMap<String, Py<PyAny>>> {
    let binding = record_view.getattr("__dict__")?;
    let dict = binding.cast::<PyDict>()?;
    let mut attrs = BTreeMap::new();
    for (key, value) in dict.iter() {
        attrs.insert(key.extract::<String>()?, value.unbind());
    }
    Ok(attrs)
}

fn extract_enrichment(
    py: Python<'_>,
    record_view: &Bound<'_, PyAny>,
    before: &BTreeMap<String, Py<PyAny>>,
    description: &str,
) -> PyResult<BTreeMap<String, String>> {
    let binding = record_view.getattr("__dict__")?;
    let after = binding.cast::<PyDict>()?;
    let mut enrichment = BTreeMap::new();

    for (key, value) in after.iter() {
        let key = key.extract::<String>()?;
        let changed = match before.get(&key) {
            Some(previous) => !python_values_equal(py, previous, &value)?,
            None => true,
        };
        if !changed {
            continue;
        }
        let candidate = match extract_supported_value(&key, &value) {
            Ok(candidate) => candidate,
            Err(err) => {
                warn!("Python filter callback '{description}' ignored enrichment: {err}");
                continue;
            }
        };
        if let Err(err) = validate_enrichment_key(&key) {
            warn!("Python filter callback '{description}' ignored enrichment: {err}");
            continue;
        }
        if let Err(err) = validate_enrichment_value(&key, &candidate) {
            warn!("Python filter callback '{description}' ignored enrichment: {err}");
            continue;
        }
        enrichment.insert(key.clone(), candidate);
        if let Err(err) = validate_enrichment_total(&enrichment) {
            enrichment.remove(&key);
            warn!("Python filter callback '{description}' ignored enrichment: {err}");
        }
    }

    Ok(enrichment)
}

fn python_values_equal(
    py: Python<'_>,
    previous: &Py<PyAny>,
    current: &Bound<'_, PyAny>,
) -> PyResult<bool> {
    previous
        .bind(py)
        .rich_compare(current, CompareOp::Eq)?
        .is_truthy()
}
