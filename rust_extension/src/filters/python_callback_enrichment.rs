//! Enrichment extraction for Python callback filters.

use std::collections::BTreeMap;

use log::warn;
use pyo3::{basic::CompareOp, prelude::*, types::PyDict};

use super::{
    super::python_callback_validation::{
        extract_supported_value,
        is_reserved_enrichment_key,
        validate_enrichment_key,
        validate_enrichment_total,
        validate_enrichment_value,
    },
    SerializedEnrichment,
    TypedEnrichment,
};

struct EnrichmentState<'description, 'py> {
    py: Python<'py>,
    description: &'description str,
    enrichment: &'description mut SerializedEnrichment,
    typed_enrichment: &'description mut TypedEnrichment,
}

fn try_validate_and_insert_enrichment(
    state: &mut EnrichmentState<'_, '_>,
    key: &str,
    value: &Bound<'_, PyAny>,
    previous: Option<&Py<PyAny>>,
) -> PyResult<bool> {
    let has_changed = previous.map_or_else(
        || Ok(true),
        |previous_value| python_values_equal(state.py, previous_value, value).map(|equal| !equal),
    )?;
    if !has_changed {
        return Ok(false);
    }

    let candidate = match extract_supported_value(key, value) {
        Ok(candidate) => candidate,
        Err(err) => {
            warn!(
                "Python filter callback '{}' ignored enrichment: {err}",
                state.description
            );
            return Ok(false);
        }
    };
    if let Err(err) = validate_enrichment_key(key) {
        warn!(
            "Python filter callback '{}' ignored enrichment: {err}",
            state.description
        );
        return Ok(false);
    }
    if let Err(err) = validate_enrichment_value(key, &candidate) {
        warn!(
            "Python filter callback '{}' ignored enrichment: {err}",
            state.description
        );
        return Ok(false);
    }

    state.enrichment.insert(key.to_owned(), candidate);
    state
        .typed_enrichment
        .insert(key.to_owned(), value.clone().unbind());
    if let Err(err) = validate_enrichment_total(state.enrichment) {
        state.enrichment.remove(key);
        state.typed_enrichment.remove(key);
        warn!(
            "Python filter callback '{}' ignored enrichment: {err}",
            state.description
        );
        return Ok(false);
    }

    Ok(true)
}

pub(super) fn extract_enrichment<'py>(
    py: Python<'py>,
    record_view: &Bound<'py, PyAny>,
    before: &BTreeMap<String, Py<PyAny>>,
    description: &str,
) -> PyResult<(SerializedEnrichment, TypedEnrichment)> {
    let binding = record_view.getattr("__dict__")?;
    let after = binding.cast::<PyDict>()?;
    let mut enrichment = SerializedEnrichment::new();
    let mut typed_enrichment = TypedEnrichment::new();
    let mut state = EnrichmentState {
        py,
        description,
        enrichment: &mut enrichment,
        typed_enrichment: &mut typed_enrichment,
    };

    for (key, value) in after.iter() {
        let key_string = key.extract::<String>()?;
        if is_reserved_enrichment_key(&key_string) {
            continue;
        }
        let previous = before.get(&key_string);
        try_validate_and_insert_enrichment(&mut state, &key_string, &value, previous)?;
    }

    Ok((enrichment, typed_enrichment))
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
