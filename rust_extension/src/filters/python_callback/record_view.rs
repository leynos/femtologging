//! Creates and reconciles the Python `LogRecord` view used by callback filters.

use std::collections::BTreeMap;

use log::warn;
use pyo3::basic::CompareOp;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::formatter::python::record_to_dict;
use crate::log_record::FemtoLogRecord;

use super::super::python_callback_validation::{
    extract_supported_value, is_reserved_enrichment_key, validate_enrichment_key,
    validate_enrichment_total, validate_enrichment_value,
};
use super::{FilterContext, SerializedEnrichment, TypedEnrichment};

/// Returns the cached callback record view or creates it for this filter chain.
pub(super) fn get_or_create_filter_record<'py>(
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
            pyo3::exceptions::PyRuntimeError::new_err("python record view not initialised")
        })?
        .bind(py)
        .clone())
}

/// Converts the Rust record and its metadata into a stdlib-compatible record.
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
    if let Some(exc_info) = record_dict.get_item("exc_info")? {
        log_record_payload.set_item("exc_info", exc_info)?;
    }
    if let Some(stack_info) = record_dict.get_item("stack_info")? {
        log_record_payload.set_item("stack_info", stack_info)?;
    }
    for (key, value) in extra.iter() {
        let key = key.extract::<String>()?;
        if is_reserved_enrichment_key(&key) {
            continue;
        }
        log_record_payload.set_item(key, value)?;
    }
    let logging = py.import("logging")?;
    logging.call_method1("makeLogRecord", (log_record_payload,))
}

/// Copies current record attributes so rejected callback changes can be discarded.
pub(super) fn snapshot_record_attrs(
    record_view: &Bound<'_, PyAny>,
) -> PyResult<BTreeMap<String, Py<PyAny>>> {
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

/// Restores the record attributes captured before a callback ran.
pub(super) fn restore_record_attrs(
    record_view: &Bound<'_, PyAny>,
    before: &BTreeMap<String, Py<PyAny>>,
) -> PyResult<()> {
    let py = record_view.py();
    let binding = record_view.getattr("__dict__")?;
    let dict = binding.cast::<PyDict>()?;
    let keys = dict.keys();
    for key in keys.iter() {
        let key = key.extract::<String>()?;
        if !before.contains_key(&key) {
            dict.del_item(&key)?;
        }
    }
    for (key, value) in before {
        dict.set_item(key, value.bind(py))?;
    }
    Ok(())
}

/// Applies validated callback enrichment to both record attributes and metadata.
pub(super) fn apply_enrichment_to_record_view(
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
        let metadata = metadata.cast::<PyDict>()?;
        if let Some(key_values) = metadata.get_item("key_values")? {
            let key_values = key_values.cast::<PyDict>()?;
            for (key, value) in typed_enrichment {
                key_values.set_item(key, value.bind(py))?;
            }
        }
    }
    debug_assert_eq!(enrichment.len(), typed_enrichment.len());
    Ok(())
}

/// Validates one changed callback attribute before adding it to enrichment maps.
fn try_validate_and_insert_enrichment(
    py: Python<'_>,
    description: &str,
    key: String,
    value: &Bound<'_, PyAny>,
    previous: Option<&Py<PyAny>>,
    enrichment: &mut SerializedEnrichment,
    typed_enrichment: &mut TypedEnrichment,
) -> PyResult<bool> {
    let has_changed = match previous {
        Some(previous) => !python_values_equal(py, previous, value)?,
        None => true,
    };
    if !has_changed {
        return Ok(false);
    }
    let candidate = match extract_supported_value(&key, value) {
        Ok(candidate) => candidate,
        Err(err) => {
            warn!("Python filter callback '{description}' ignored enrichment: {err}");
            return Ok(false);
        }
    };
    if let Err(err) = validate_enrichment_key(&key) {
        warn!("Python filter callback '{description}' ignored enrichment: {err}");
        return Ok(false);
    }
    if let Err(err) = validate_enrichment_value(&key, &candidate) {
        warn!("Python filter callback '{description}' ignored enrichment: {err}");
        return Ok(false);
    }
    enrichment.insert(key.clone(), candidate);
    typed_enrichment.insert(key.clone(), value.clone().unbind());
    if let Err(err) = validate_enrichment_total(enrichment) {
        enrichment.remove(&key);
        typed_enrichment.remove(&key);
        warn!("Python filter callback '{description}' ignored enrichment: {err}");
        return Ok(false);
    }
    Ok(true)
}

/// Extracts changed, valid attributes from the callback record.
pub(super) fn extract_enrichment(
    py: Python<'_>,
    record_view: &Bound<'_, PyAny>,
    before: &BTreeMap<String, Py<PyAny>>,
    description: &str,
) -> PyResult<(SerializedEnrichment, TypedEnrichment)> {
    let binding = record_view.getattr("__dict__")?;
    let after = binding.cast::<PyDict>()?;
    let mut enrichment = SerializedEnrichment::new();
    let mut typed_enrichment = TypedEnrichment::new();
    for (key, value) in after.iter() {
        let key = key.extract::<String>()?;
        if is_reserved_enrichment_key(&key) {
            continue;
        }
        let previous = before.get(&key);
        let _ = try_validate_and_insert_enrichment(
            py,
            description,
            key,
            &value,
            previous,
            &mut enrichment,
            &mut typed_enrichment,
        )?;
    }
    Ok((enrichment, typed_enrichment))
}

/// Compares Python values using their native equality semantics.
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
