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
use pyo3::types::PyDict;

use crate::log_record::FemtoLogRecord;
use crate::macros::{AsPyDict, dict_into_py};
use crate::python::fq_py_type;

use super::python_callback_warning::CallbackFailureWarner;
use super::{FemtoFilter, FilterBuildError, FilterBuilderTrait, FilterContext, FilterDecision};

#[path = "python_callback/record_view.rs"]
mod record_view;
use record_view::{
    apply_enrichment_to_record_view, extract_enrichment, get_or_create_filter_record,
    restore_record_attrs, snapshot_record_attrs,
};

/// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
type SerializedEnrichment = BTreeMap<String, String>;
/// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
type TypedEnrichment = BTreeMap<String, Py<PyAny>>;

/// A filter backed by a Python callable or `logging.Filter`-style object.
#[derive(Clone, Debug)]
pub struct PythonCallbackFilter {
    /// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
    callback: Arc<Mutex<Py<PyAny>>>,
    /// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
    description: String,
    /// Shared rate limiter and recursion guard for callback-failure warnings.
    failure_warner: CallbackFailureWarner,
}

impl PythonCallbackFilter {
    /// Build a new Python callback filter from a validated Python object.
    pub fn new(callback: Py<PyAny>, description: String) -> Self {
        Self {
            callback: Arc::new(Mutex::new(callback)),
            description,
            failure_warner: CallbackFailureWarner::default(),
        }
    }

    /// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
    fn decision_with_context(
        &self,
        record: &FemtoLogRecord,
        context: &mut FilterContext,
    ) -> Result<FilterDecision, PyErr> {
        Python::attach(|py| {
            let record_view = get_or_create_filter_record(py, record, context)?;
            let before = snapshot_record_attrs(&record_view)?;
            let outcome = (|| -> Result<FilterDecision, PyErr> {
                let result = self.invoke(py, &record_view)?;
                let accepted = result.is_truthy()?;
                let (enrichment, typed_enrichment) =
                    extract_enrichment(py, &record_view, &before, &self.description)?;
                restore_record_attrs(&record_view, &before)?;
                if !accepted {
                    return Ok(FilterDecision::accept(false));
                }
                apply_enrichment_to_record_view(&record_view, &enrichment, &typed_enrichment)?;
                Ok(FilterDecision {
                    accepted: true,
                    callback_failed: false,
                    enrichment,
                })
            })();

            if outcome.is_err() {
                context.python_record_view = None;
                let _ = restore_record_attrs(&record_view, &before);
            }
            outcome
        })
    }

    /// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
    fn invoke<'py>(
        &self,
        py: Python<'py>,
        record_view: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let callback = self
            .callback
            .lock()
            .map_err(|_| {
                pyo3::exceptions::PyRuntimeError::new_err("python callback filter mutex poisoned")
            })?
            .clone_ref(py);
        let callback = callback.bind(py);
        if callback.is_callable() {
            callback.call1((record_view,))
        } else {
            callback.getattr("filter")?.call1((record_view,))
        }
    }

    /// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
    pub(crate) fn description(&self) -> &str {
        &self.description
    }
}
impl FemtoFilter for PythonCallbackFilter {
    fn decision(&self, record: &mut FemtoLogRecord, context: &mut FilterContext) -> FilterDecision {
        match self.decision_with_context(record, context) {
            Ok(decision) => decision,
            Err(_) => {
                self.failure_warner.report(|failure_count| {
                    warn!("Python filter callback failed; records dropped: {failure_count}");
                });
                FilterDecision::callback_failure()
            }
        }
    }
}

/// Builder for a Python callback filter.
#[pyclass(from_py_object)]
#[derive(Clone, Debug)]
pub struct PythonCallbackFilterBuilder {
    /// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
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
    /// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
    #[new]
    #[pyo3(text_signature = "(callback, /)")]
    fn py_new(callback: Bound<'_, PyAny>) -> PyResult<Self> {
        Self::from_callback_obj(callback)
    }

    /// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
    fn as_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.as_pydict(py)
    }
}

/// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
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
