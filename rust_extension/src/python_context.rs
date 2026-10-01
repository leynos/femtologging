//! Python-side conversion for scoped and inline structured logging fields.
//!
//! This adapter validates Python mappings and scalar objects before copying
//! them into the Rust-owned context representation.

use std::collections::BTreeMap;

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyFloat, PyInt, PyString, PyTuple};

use crate::log_context::ContextBudget;

/// Convert a Python mapping into the validated scalar representation used for
/// both scoped and inline structured fields.
///
/// Only string keys and values of the supported built-in scalar types (`str`,
/// `int`, `float`, `bool`, or `None`) are accepted. Objects that merely offer
/// conversion protocols such as `__float__` are rejected. Size limits are
/// checked while iterating, before converted strings are retained.
///
/// # Errors
///
/// Returns `TypeError` for an unsupported mapping shape or scalar type, and
/// `ValueError` when a key, value, key count, or aggregate size exceeds the
/// structured-context limits.
pub(crate) fn extract_python_context_map(
    context: &Bound<'_, PyAny>,
) -> PyResult<BTreeMap<String, String>> {
    let items = context.call_method0("items").map_err(|_| {
        PyTypeError::new_err("context must be a mapping[str, str|int|float|bool|None]")
    })?;
    let mut result = BTreeMap::new();
    let mut budget = ContextBudget::default();
    for item in items.try_iter().map_err(|_| {
        PyTypeError::new_err("context must be a mapping[str, str|int|float|bool|None]")
    })? {
        let item = item?;
        let pair = item
            .cast::<PyTuple>()
            .map_err(|_| PyTypeError::new_err("context items must contain key-value pairs"))?;
        if pair.len() != 2 {
            return Err(PyTypeError::new_err(
                "context items must contain key-value pairs",
            ));
        }
        let raw_key = pair.get_item(0)?;
        let key = raw_key
            .cast::<PyString>()
            .map_err(|_| PyTypeError::new_err("context keys must be strings"))?
            .to_str()
            .map_err(|_| PyTypeError::new_err("context keys must be strings"))?;

        with_python_context_value(&pair.get_item(1)?, |value| {
            let previous_value = result.get(key).map(String::as_str);
            budget
                .validate_next(key, value, previous_value)
                .map_err(|error| PyValueError::new_err(error.to_string()))?;
            result.insert(key.to_owned(), value.to_owned());
            Ok(())
        })?;
    }
    Ok(result)
}

/// Invoke `consumer` with a supported Python scalar's borrowed string form.
///
/// This prevents Python conversion protocols from admitting objects that only
/// implement `__str__` or `__float__`.
fn with_python_context_value<T>(
    raw_value: &Bound<'_, PyAny>,
    consumer: impl FnOnce(&str) -> PyResult<T>,
) -> PyResult<T> {
    if raw_value.is_none() {
        return consumer("None");
    }
    if [
        raw_value.is_instance_of::<PyBool>(),
        raw_value.is_instance_of::<PyInt>(),
        raw_value.is_instance_of::<PyFloat>(),
    ]
    .contains(&true)
    {
        let value = raw_value.str()?;
        return consumer(value.to_str()?);
    }
    if let Ok(value) = raw_value.cast::<PyString>() {
        return consumer(value.to_str()?);
    }
    Err(PyTypeError::new_err(
        "context values must be str, int, float, bool, or None",
    ))
}

#[cfg(test)]
mod tests {
    //! Tests for Python mapping conversion and its size limits.

    use std::collections::BTreeMap;

    use pyo3::exceptions::PyValueError;
    use pyo3::prelude::*;
    use pyo3::types::PyDict;

    use crate::log_context::{MAX_CONTEXT_KEYS, MAX_KEY_BYTES, MAX_VALUE_BYTES};

    use super::extract_python_context_map;

    /// Assert that conversion rejects a mapping which exceeds a size limit.
    fn assert_context_size_rejected(context: &Bound<'_, PyAny>) {
        let error = extract_python_context_map(context).expect_err("context should be rejected");
        assert!(error.is_instance_of::<PyValueError>(context.py()));
    }

    /// Reject overlong keys and values, excessive key counts, and total size.
    #[test]
    fn extraction_rejects_each_context_size_limit() {
        Python::attach(|py| {
            let context = PyDict::new(py);
            context
                .set_item("k".repeat(MAX_KEY_BYTES + 1), "v")
                .expect("context item should be set");
            assert_context_size_rejected(context.as_any());

            context.clear();
            for index in 0..=MAX_CONTEXT_KEYS {
                context
                    .set_item(format!("k{index}"), "v")
                    .expect("context item should be set");
            }
            assert_context_size_rejected(context.as_any());

            context.clear();
            context
                .set_item("key", "v".repeat(MAX_VALUE_BYTES + 1))
                .expect("context item should be set");
            assert_context_size_rejected(context.as_any());

            context.clear();
            for index in 0..16usize {
                context
                    .set_item(format!("k{index:02}"), "v".repeat(MAX_VALUE_BYTES))
                    .expect("context item should be set");
            }
            assert_context_size_rejected(context.as_any());
        });
    }

    /// Keep the last value for repeated keys without charging overwritten data.
    #[test]
    fn extraction_keeps_only_the_final_value_for_duplicate_keys() {
        Python::attach(|py| {
            let context = py
                .eval(
                    c"type('M', (), {'items': lambda _: [('shared', 'v' * 1024)] * 17 + [('shared', 'last')]})()",
                    None,
                    None,
                )
                .expect("duplicate-items instance should be created");
            let converted =
                extract_python_context_map(&context).expect("duplicate keys should be accepted");
            assert_eq!(
                converted,
                BTreeMap::from([(String::from("shared"), String::from("last"))])
            );
        });
    }
}
