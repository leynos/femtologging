//! Helpers for traceback frame extraction unit tests.
//!
//! These utilities build Python objects that resemble `traceback.FrameSummary`
//! values so unit tests can exercise the conversion logic in
//! [`crate::traceback_frames`].

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use std::collections::BTreeMap;

use crate::traceback_frames::{extract_frames_from_stack_summary, extract_locals_dict};

// Re-exported so `use crate::test_utils::traceback_test_helpers::*;` continues
// to bring the frame assertion helpers into scope.
pub use crate::test_utils::frame_assertion_helpers::*;

/// Create a `types.SimpleNamespace` object from a [`PyDict`].
pub fn create_simple_namespace<'py>(
    py: Python<'py>,
    dict: &Bound<'py, PyDict>,
) -> PyResult<Bound<'py, PyAny>> {
    py.import("types")?
        .getattr("SimpleNamespace")?
        .call((), Some(dict))
}

/// Look up a name in the `builtins` module, such as an exception type.
pub fn builtin_type<'py>(py: Python<'py>, type_name: &str) -> PyResult<Bound<'py, PyAny>> {
    py.import("builtins")?.getattr(type_name)
}

/// Create a built-in exception instance from `type_name` with the given arguments.
///
/// Useful where a test needs an exception type other than `ValueError`, such as
/// `KeyError`, without repeating the `builtins` lookup at every call site.
pub fn create_builtin_exception<'py>(
    py: Python<'py>,
    type_name: &str,
    args: impl pyo3::call::PyCallArgs<'py>,
) -> PyResult<Bound<'py, PyAny>> {
    builtin_type(py, type_name)?.call1(args)
}

/// Create a `ValueError` instance carrying the given message.
pub fn create_value_error<'py>(py: Python<'py>, message: &str) -> PyResult<Bound<'py, PyAny>> {
    create_builtin_exception(py, "ValueError", (message,))
}

/// Create a `BaseException` instance with no arguments.
pub fn create_base_exception<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
    builtin_type(py, "BaseException")?.call0()
}

/// Create a frame dict pre-populated with the three required frame fields.
///
/// Tests that only care about optional or malformed fields can start from this
/// dict and override or add entries as needed.
pub fn base_frame_dict(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
    let frame_dict = PyDict::new(py);
    frame_dict.set_item("filename", "test.py")?;
    frame_dict.set_item("lineno", 1)?;
    frame_dict.set_item("name", "func")?;
    Ok(frame_dict)
}

/// Create a frame dict with locals for testing [`crate::traceback_frames::extract_locals_dict`].
pub fn create_frame_dict_with_locals<'py>(
    py: Python<'py>,
    locals_dict: &Bound<'py, PyDict>,
) -> PyResult<Bound<'py, PyDict>> {
    let frame_dict = base_frame_dict(py)?;
    frame_dict.set_item("locals", locals_dict)?;
    Ok(frame_dict)
}

/// Builder for creating mock FrameSummary-like objects in tests.
///
/// Groups related frame attributes and provides chainable setters for optional
/// fields, reducing parameter count and improving readability at call sites.
pub struct MockFrameBuilder {
    filename: String,
    lineno: u32,
    name: String,
    end_lineno: Option<u32>,
    colno: Option<u32>,
    end_colno: Option<u32>,
    line: Option<String>,
    locals: Option<Vec<(String, String)>>,
}

impl MockFrameBuilder {
    /// Create a new builder with required fields.
    pub fn new(filename: impl Into<String>, lineno: u32, name: impl Into<String>) -> Self {
        Self {
            filename: filename.into(),
            lineno,
            name: name.into(),
            end_lineno: None,
            colno: None,
            end_colno: None,
            line: None,
            locals: None,
        }
    }

    /// Set the end line number.
    pub fn end_lineno(mut self, value: u32) -> Self {
        self.end_lineno = Some(value);
        self
    }

    /// Set the column offset.
    pub fn colno(mut self, value: u32) -> Self {
        self.colno = Some(value);
        self
    }

    /// Set the end column offset.
    pub fn end_colno(mut self, value: u32) -> Self {
        self.end_colno = Some(value);
        self
    }

    /// Set the source line.
    pub fn line(mut self, value: impl Into<String>) -> Self {
        self.line = Some(value.into());
        self
    }

    /// Set the locals dictionary entries.
    pub fn locals(mut self, entries: &[(&str, &str)]) -> Self {
        self.locals = Some(
            entries
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        );
        self
    }

    /// Build the mock frame as a Python `SimpleNamespace` object.
    pub fn build<'py>(self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let dict = PyDict::new(py);
        dict.set_item("filename", &self.filename)?;
        dict.set_item("lineno", self.lineno)?;
        dict.set_item("name", &self.name)?;

        if let Some(v) = self.end_lineno {
            dict.set_item("end_lineno", v)?;
        }
        if let Some(v) = self.colno {
            dict.set_item("colno", v)?;
        }
        if let Some(v) = self.end_colno {
            dict.set_item("end_colno", v)?;
        }
        if let Some(v) = &self.line {
            dict.set_item("line", v)?;
        }
        if let Some(entries) = &self.locals {
            let locals_dict = PyDict::new(py);
            for (k, v) in entries {
                locals_dict.set_item(k, v)?;
            }
            dict.set_item("locals", locals_dict)?;
        }

        create_simple_namespace(py, &dict)
    }
}

/// Key descriptor for parameterised locals extraction tests.
///
/// Keys starting with digits are parsed as integers; others are strings.
#[derive(Debug, Clone)]
pub struct LocalEntry {
    key: &'static str,
    value: &'static str,
}

impl LocalEntry {
    pub const fn new(key: &'static str, value: &'static str) -> Self {
        Self { key, value }
    }

    pub const fn key(&self) -> &'static str {
        self.key
    }

    pub const fn value(&self) -> &'static str {
        self.value
    }

    /// Returns true if the key should be inserted as an integer.
    ///
    /// This is a simple heuristic that only checks whether the first character
    /// is an ASCII digit. It is sufficient for the test cases in this module,
    /// which use single-digit integer keys like "1" or "123".
    pub fn is_int_key(&self) -> bool {
        self.key.chars().next().is_some_and(|c| c.is_ascii_digit())
    }
}

/// Create a Python object whose `__repr__` raises an exception.
///
/// Useful for testing repr failure handling in locals extraction.
pub fn create_bad_repr_object<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
    let globals = PyDict::new(py);
    py.run(
        c"class BadRepr:\n    def __repr__(self): raise ValueError('boom')",
        Some(&globals),
        None,
    )?;
    py.eval(c"BadRepr()", Some(&globals), None)
}

/// Add a bad repr entry to the given dictionary.
///
/// Creates a Python object whose `__repr__` raises an exception and inserts it
/// with the given key.
pub fn add_bad_repr_entry(locals_dict: &Bound<'_, PyDict>, key: &str) -> PyResult<()> {
    let bad_repr_obj = create_bad_repr_object(locals_dict.py())?;
    locals_dict.set_item(key, bad_repr_obj)
}

/// Arrange a single-frame stack summary from `dict` and return the error that
/// frame extraction raises for it.
///
/// Returns an error when the arrangement (namespace or list creation) fails, or
/// when extraction unexpectedly succeeds.
pub fn frame_extraction_error(dict: &Bound<'_, PyDict>) -> PyResult<PyErr> {
    let py = dict.py();
    let frame = create_simple_namespace(py, dict)?;
    let list = PyList::new(py, &[frame])?;
    match extract_frames_from_stack_summary(list.as_any()) {
        Ok(frames) => Err(PyRuntimeError::new_err(format!(
            "frame extraction should fail, but yielded {} frame(s)",
            frames.len()
        ))),
        Err(err) => Ok(err),
    }
}

/// Assert that an error's rendered message contains the expected substring.
#[track_caller]
pub fn assert_error_message_contains(err: &PyErr, expected_substr: &str) {
    let err_text = err.to_string();
    assert!(
        err_text.contains(expected_substr),
        "expected error containing {expected_substr:?}, got {err_text:?}"
    );
}

/// Assert that extracting a frame from the provided dict fails with an error
/// containing the expected substring.
///
/// Returns an error when the arrangement fails; the assertion itself panics
/// with the calling test's location.
#[track_caller]
pub fn assert_frame_extraction_error_contains(
    dict: &Bound<'_, PyDict>,
    expected_substr: &str,
) -> PyResult<()> {
    let err = frame_extraction_error(dict)?;
    assert_error_message_contains(&err, expected_substr);
    Ok(())
}

/// Extracted locals for a frame built around `locals_dict`.
///
/// Arranges the frame dict and namespace, then calls
/// [`crate::traceback_frames::extract_locals_dict`], propagating arrangement
/// failures to the caller.
pub fn extract_locals_for_dict(
    locals_dict: &Bound<'_, PyDict>,
) -> PyResult<Option<BTreeMap<String, String>>> {
    let py = locals_dict.py();
    let frame_dict = create_frame_dict_with_locals(py, locals_dict)?;
    let frame = create_simple_namespace(py, &frame_dict)?;
    Ok(extract_locals_dict(&frame))
}

/// Compare extracted locals against the expected entries.
///
/// Returns `Err` with a human-readable explanation when the two disagree. This
/// is a pure query over already-extracted data, so it never panics.
pub fn compare_locals(
    actual: Option<&BTreeMap<String, String>>,
    expected: Option<&[(&str, &str)]>,
) -> Result<(), String> {
    match (expected, actual) {
        (Some(expected_entries), Some(locals)) => {
            if locals.len() != expected_entries.len() {
                return Err(format!(
                    "expected {} locals entries, found {} ({locals:?})",
                    expected_entries.len(),
                    locals.len()
                ));
            }
            for (key, value) in expected_entries {
                let found = locals.get(*key).map(String::as_str);
                if found != Some(*value) {
                    return Err(format!("key {key:?} should be {value:?}, found {found:?}"));
                }
            }
            Ok(())
        }
        (Some(_), None) => Err("locals should be extracted, found None".to_string()),
        (None, Some(locals)) => Err(format!("locals should be None, found {locals:?}")),
        (None, None) => Ok(()),
    }
}

/// Assert the result of `extract_locals_dict` against expected entries.
///
/// Builds a frame from the provided locals dict, calls `extract_locals_dict`,
/// and verifies that the result matches the expected entries (or is `None`).
/// Returns an error when the arrangement (frame construction) fails; the
/// assertion itself panics with the calling test's location.
#[track_caller]
pub fn assert_locals_extraction_result(
    locals_dict: &Bound<'_, PyDict>,
    expected: Option<&[(&str, &str)]>,
    description: &str,
) -> PyResult<()> {
    let actual = extract_locals_for_dict(locals_dict)?;
    if let Err(mismatch) = compare_locals(actual.as_ref(), expected) {
        panic!("{description}: {mismatch}");
    }
    Ok(())
}

/// Populate a PyDict with LocalEntry items, inserting integer keys for entries
/// where `is_int_key()` returns true and the key successfully parses as `i32`.
///
/// Falls back to inserting as a string key if parsing fails (e.g., overflow).
pub fn populate_locals_dict_from_entries(
    locals_dict: &Bound<'_, PyDict>,
    entries: &[LocalEntry],
) -> PyResult<()> {
    for entry in entries {
        if entry.is_int_key() {
            if let Ok(int_key) = entry.key().parse::<i32>() {
                locals_dict.set_item(int_key, entry.value())?;
                continue;
            }
        }
        // Fallback: insert as string key (either not an int key, or parsing failed)
        locals_dict.set_item(entry.key(), entry.value())?;
    }
    Ok(())
}
