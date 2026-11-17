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
    let payload = build_log_record_payload(py, record, record_dict)?;
    let logging = py.import("logging")?;
    logging.call_method1("makeLogRecord", (payload,))
}

/// Build the payload passed to `logging.makeLogRecord`.
fn build_log_record_payload<'py>(
    py: Python<'py>,
    record: &FemtoLogRecord,
    record_dict: &Bound<'py, PyDict>,
) -> PyResult<Bound<'py, PyDict>> {
    let (metadata, extra) = record_metadata_view(record_dict)?;
    let payload = PyDict::new(py);
    populate_log_record_core(record, &metadata, &payload)?;
    copy_optional_record_fields(record_dict, &payload)?;
    copy_extra_fields(&extra, &payload)?;
    Ok(payload)
}

/// Return the record's metadata mapping and its structured enrichment values.
fn record_metadata_view<'py>(
    record_dict: &Bound<'py, PyDict>,
) -> PyResult<(Bound<'py, PyDict>, Bound<'py, PyDict>)> {
    let metadata = record_dict
        .get_item("metadata")?
        .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("record dict missing metadata"))?;
    let metadata = metadata.cast::<PyDict>()?.clone();
    let extra = metadata.get_item("key_values")?.ok_or_else(|| {
        pyo3::exceptions::PyRuntimeError::new_err("record dict missing key_values")
    })?;
    let extra = extra.cast::<PyDict>()?.clone();
    Ok((metadata, extra))
}

/// Populate the core stdlib-compatible fields, including metadata.
fn populate_log_record_core(
    record: &FemtoLogRecord,
    metadata: &Bound<'_, PyDict>,
    payload: &Bound<'_, PyDict>,
) -> PyResult<()> {
    payload.set_item("name", record.logger())?;
    payload.set_item("msg", record.message())?;
    payload.set_item("levelname", record.level_str())?;
    payload.set_item("levelno", u8::from(record.level()))?;
    payload.set_item("pathname", &record.metadata().filename)?;
    payload.set_item("filename", &record.metadata().filename)?;
    payload.set_item("module", &record.metadata().module_path)?;
    payload.set_item("lineno", record.metadata().line_number)?;
    payload.set_item("logger", record.logger())?;
    payload.set_item("level", record.level_str())?;
    payload.set_item("message", record.message())?;
    payload.set_item("metadata", metadata)?;
    payload.set_item("args", PyList::empty(payload.py()))?;
    Ok(())
}

/// Copy the optional exception and stack payloads from the converted record.
fn copy_optional_record_fields(
    record_dict: &Bound<'_, PyDict>,
    payload: &Bound<'_, PyDict>,
) -> PyResult<()> {
    copy_record_item(record_dict, payload, "exc_info")?;
    copy_record_item(record_dict, payload, "stack_info")
}

/// Copy one optional record field when the converted record provides it.
fn copy_record_item(
    record_dict: &Bound<'_, PyDict>,
    payload: &Bound<'_, PyDict>,
    key: &str,
) -> PyResult<()> {
    if let Some(value) = record_dict.get_item(key)? {
        payload.set_item(key, value)?;
    }
    Ok(())
}

/// Merge non-reserved enrichment values into the payload.
fn copy_extra_fields(extra: &Bound<'_, PyDict>, payload: &Bound<'_, PyDict>) -> PyResult<()> {
    for (key, value) in extra.iter() {
        let key = key.extract::<String>()?;
        if is_reserved_enrichment_key(&key) {
            continue;
        }
        payload.set_item(key, value)?;
    }
    Ok(())
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

/// Shares the callback snapshot and accumulation maps during enrichment extraction.
struct EnrichmentInsertionContext<'py, 'a> {
    /// Python token used for value comparisons.
    py: Python<'py>,
    /// Callback label included in rejected-enrichment warnings.
    description: &'a str,
    /// Original callback attributes used to detect changed values.
    before: &'a BTreeMap<String, Py<PyAny>>,
    /// String values accepted for Rust-side structured metadata.
    enrichment: &'a mut SerializedEnrichment,
    /// Original Python values retained for the callback record view.
    typed_enrichment: &'a mut TypedEnrichment,
}

/// Validates one changed callback attribute before adding it to enrichment maps.
fn try_validate_and_insert_enrichment(
    context: &mut EnrichmentInsertionContext<'_, '_>,
    key: String,
    value: &Bound<'_, PyAny>,
) -> PyResult<bool> {
    let previous = context.before.get(&key);
    let has_changed = match previous {
        Some(previous) => !python_values_equal(context.py, previous, value)?,
        None => true,
    };
    if !has_changed {
        return Ok(false);
    }
    let candidate = match extract_supported_value(&key, value) {
        Ok(candidate) => candidate,
        Err(err) => {
            warn!(
                "Python filter callback '{}' ignored enrichment: {err}",
                context.description
            );
            return Ok(false);
        }
    };
    if let Err(err) = validate_enrichment_key(&key) {
        warn!(
            "Python filter callback '{}' ignored enrichment: {err}",
            context.description
        );
        return Ok(false);
    }
    if let Err(err) = validate_enrichment_value(&key, &candidate) {
        warn!(
            "Python filter callback '{}' ignored enrichment: {err}",
            context.description
        );
        return Ok(false);
    }
    context.enrichment.insert(key.clone(), candidate);
    context
        .typed_enrichment
        .insert(key.clone(), value.clone().unbind());
    if let Err(err) = validate_enrichment_total(context.enrichment) {
        context.enrichment.remove(&key);
        context.typed_enrichment.remove(&key);
        warn!(
            "Python filter callback '{}' ignored enrichment: {err}",
            context.description
        );
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
    {
        let mut context = EnrichmentInsertionContext {
            py,
            description,
            before,
            enrichment: &mut enrichment,
            typed_enrichment: &mut typed_enrichment,
        };
        for (key, value) in after.iter() {
            let key = key.extract::<String>()?;
            if is_reserved_enrichment_key(&key) {
                continue;
            }
            let _ = try_validate_and_insert_enrichment(&mut context, key, &value)?;
        }
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

#[cfg(test)]
mod tests {
    //! Tests for the log record payload helpers.

    use pyo3::Python;
    use pyo3::types::{PyDict, PyDictMethods};

    use super::{copy_extra_fields, record_metadata_view};

    #[test]
    fn record_metadata_view_requires_metadata_and_key_values() {
        Python::attach(|py| {
            let record_dict = PyDict::new(py);
            let err = match record_metadata_view(&record_dict) {
                Ok(_) => panic!("metadata must be required"),
                Err(err) => err,
            };
            assert_eq!(
                err.to_string(),
                "RuntimeError: record dict missing metadata"
            );

            let metadata = PyDict::new(py);
            record_dict
                .set_item("metadata", &metadata)
                .expect("set metadata");
            let err = match record_metadata_view(&record_dict) {
                Ok(_) => panic!("key_values must be required"),
                Err(err) => err,
            };
            assert_eq!(
                err.to_string(),
                "RuntimeError: record dict missing key_values"
            );
        });
    }

    #[test]
    fn copy_extra_fields_skips_reserved_keys() {
        Python::attach(|py| {
            let extra = PyDict::new(py);
            extra.set_item("request_id", "abc").expect("set extra");
            extra.set_item("message", "ignored").expect("set extra");
            let payload = PyDict::new(py);
            copy_extra_fields(&extra, &payload).expect("copy extras");

            assert!(payload.get_item("request_id").expect("lookup").is_some());
            assert!(payload.get_item("message").expect("lookup").is_none());
        });
    }
}
