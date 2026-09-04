//! Validation helpers for Python callback filter enrichment.

use pyo3::prelude::*;
use pyo3::types::{PyBool, PyFloat, PyInt, PyString};

use crate::python::fq_py_type;

/// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
const MAX_ENRICHMENT_KEYS: usize = 64;
/// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
const MAX_KEY_BYTES: usize = 64;
/// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
const MAX_VALUE_BYTES: usize = 1024;
/// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
const MAX_TOTAL_BYTES: usize = 16 * 1024;

/// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
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

/// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
const RESERVED_FEMTO_KEYS: &[&str] = &[
    "logger",
    "level",
    "metadata",
    "module_path",
    "line_number",
    "timestamp",
    "thread_id",
    "thread_name",
    "key_values",
];

/// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
pub(crate) fn is_reserved_enrichment_key(key: &str) -> bool {
    RESERVED_STD_RECORD_KEYS.contains(&key) || RESERVED_FEMTO_KEYS.contains(&key)
}

/// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
#[derive(Debug, thiserror::Error)]
pub(crate) enum EnrichmentError {
    /// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
    #[error("enrichment key must not be empty")]
    EmptyKey,
    /// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
    #[error("enrichment key '{key}' exceeds {limit} UTF-8 bytes")]
    KeyTooLong {
        /// Identifies the Python-provided key rejected before it reaches record metadata.
        key: String,
        /// States the UTF-8 byte limit used to keep enriched records bounded.
        limit: usize,
    },
    /// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
    #[error("enrichment key '{key}' is reserved")]
    ReservedKey {
        /// Identifies the Python key that would collide with a protected record field.
        key: String,
    },
    /// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
    #[error("enrichment value for '{key}' exceeds {limit} UTF-8 bytes")]
    ValueTooLong {
        /// Identifies the key whose Python value exceeded the transport-safe bound.
        key: String,
        /// States the UTF-8 byte limit used to preserve bounded queue payloads.
        limit: usize,
    },
    /// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
    #[error("enrichment supports at most {limit} keys")]
    TooManyKeys {
        /// States the maximum number of enrichment entries accepted for one record.
        limit: usize,
    },
    /// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
    #[error("enrichment total exceeds {limit} bytes")]
    TotalTooLarge {
        /// States the maximum combined metadata size permitted for one record.
        limit: usize,
    },
    /// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
    #[error("enrichment key '{key}' has unsupported Python type {python_type}")]
    UnsupportedValueType {
        /// Identifies the key whose Python value cannot be serialised into record metadata.
        key: String,
        /// Captures the rejected Python type for an actionable boundary error.
        python_type: String,
    },
}

/// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
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

/// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
pub(crate) fn validate_enrichment_value(key: &str, value: &str) -> Result<(), EnrichmentError> {
    if value.len() > MAX_VALUE_BYTES {
        return Err(EnrichmentError::ValueTooLong {
            key: key.to_owned(),
            limit: MAX_VALUE_BYTES,
        });
    }
    Ok(())
}

/// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
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

/// Bridges Python calls into Rust while maintaining PyO3 ownership and exception propagation boundaries.
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
    if is_supported_scalar(value) {
        return Ok(value
            .str()
            .map_err(|_| unsupported())?
            .to_str()
            .map_err(|_| unsupported())?
            .to_owned());
    }

    Err(unsupported())
}

/// Report whether the value is one of the scalar Python types accepted by
/// enrichment (bool, str, int, or float).
fn is_supported_scalar(value: &Bound<'_, PyAny>) -> bool {
    [
        value.is_instance_of::<PyBool>(),
        value.is_instance_of::<PyString>(),
        value.is_instance_of::<PyInt>(),
        value.is_instance_of::<PyFloat>(),
    ]
    .contains(&true)
}
