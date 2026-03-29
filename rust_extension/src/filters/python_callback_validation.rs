//! Validation helpers for Python callback filter enrichment.

use pyo3::prelude::*;

use crate::python::fq_py_type;

const MAX_ENRICHMENT_KEYS: usize = 64;
const MAX_KEY_BYTES: usize = 64;
const MAX_VALUE_BYTES: usize = 1024;
const MAX_TOTAL_BYTES: usize = 16 * 1024;

const RESERVED_STD_RECORD_KEYS: &[&str] = &[
    "name",
    "msg",
    "args",
    "levelname",
    "levelno",
    "pathname",
    "filename",
    "module",
    "exc_info",
    "exc_text",
    "stack_info",
    "lineno",
    "funcName",
    "created",
    "msecs",
    "relativeCreated",
    "thread",
    "threadName",
    "process",
    "processName",
    "message",
    "asctime",
    "taskName",
];

const RESERVED_FEMTO_KEYS: &[&str] = &["logger", "level", "metadata"];

pub(crate) fn is_reserved_enrichment_key(key: &str) -> bool {
    RESERVED_STD_RECORD_KEYS.contains(&key) || RESERVED_FEMTO_KEYS.contains(&key)
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum EnrichmentError {
    #[error("enrichment key must not be empty")]
    EmptyKey,
    #[error("enrichment key '{key}' exceeds {limit} UTF-8 bytes")]
    KeyTooLong { key: String, limit: usize },
    #[error("enrichment key '{key}' is reserved")]
    ReservedKey { key: String },
    #[error("enrichment value for '{key}' exceeds {limit} UTF-8 bytes")]
    ValueTooLong { key: String, limit: usize },
    #[error("enrichment supports at most {limit} keys")]
    TooManyKeys { limit: usize },
    #[error("enrichment total exceeds {limit} bytes")]
    TotalTooLarge { limit: usize },
    #[error("enrichment key '{key}' has unsupported Python type {python_type}")]
    UnsupportedValueType { key: String, python_type: String },
}

pub(crate) fn validate_enrichment_key(key: &str) -> Result<(), EnrichmentError> {
    if key.is_empty() {
        return Err(EnrichmentError::EmptyKey);
    }
    if key.len() > MAX_KEY_BYTES {
        return Err(EnrichmentError::KeyTooLong {
            key: key.to_owned(),
            limit: MAX_KEY_BYTES,
        });
    }
    if is_reserved_enrichment_key(key) {
        return Err(EnrichmentError::ReservedKey {
            key: key.to_owned(),
        });
    }
    Ok(())
}

pub(crate) fn validate_enrichment_value(key: &str, value: &str) -> Result<(), EnrichmentError> {
    if value.len() > MAX_VALUE_BYTES {
        return Err(EnrichmentError::ValueTooLong {
            key: key.to_owned(),
            limit: MAX_VALUE_BYTES,
        });
    }
    Ok(())
}

pub(crate) fn validate_enrichment_total(
    enrichment: &std::collections::BTreeMap<String, String>,
) -> Result<(), EnrichmentError> {
    if enrichment.len() > MAX_ENRICHMENT_KEYS {
        return Err(EnrichmentError::TooManyKeys {
            limit: MAX_ENRICHMENT_KEYS,
        });
    }
    let total_bytes = enrichment
        .iter()
        .fold(0usize, |sum, (key, value)| sum + key.len() + value.len());
    if total_bytes > MAX_TOTAL_BYTES {
        return Err(EnrichmentError::TotalTooLarge {
            limit: MAX_TOTAL_BYTES,
        });
    }
    Ok(())
}

pub(crate) fn extract_supported_value(
    key: &str,
    value: &Bound<'_, PyAny>,
) -> Result<String, EnrichmentError> {
    let unsupported = || EnrichmentError::UnsupportedValueType {
        key: key.to_owned(),
        python_type: fq_py_type(value),
    };

    if value.is_none() {
        return Ok("None".to_owned());
    }
    if value.extract::<bool>().is_ok()
        || value.extract::<String>().is_ok()
        || value.extract::<i128>().is_ok()
        || value.extract::<u128>().is_ok()
        || value.extract::<f64>().is_ok()
    {
        return Ok(value
            .str()
            .map_err(|_| unsupported())?
            .to_str()
            .map_err(|_| unsupported())?
            .to_owned());
    }

    Err(unsupported())
}
