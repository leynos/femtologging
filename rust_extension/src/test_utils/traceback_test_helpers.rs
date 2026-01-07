//! Helpers for traceback frame extraction unit tests.
//!
//! These utilities build Python objects that resemble `traceback.FrameSummary`
//! values so unit tests can exercise the conversion logic in
//! [`crate::traceback_frames`].

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::traceback_frames::extract_frames_from_stack_summary;

/// Create a `types.SimpleNamespace` object from a [`PyDict`].
pub fn create_simple_namespace<'py>(
    py: Python<'py>,
    dict: &Bound<'py, PyDict>,
) -> Bound<'py, PyAny> {
    let types = py.import("types").expect("types module should exist");
    types
        .getattr("SimpleNamespace")
        .expect("SimpleNamespace should exist")
        .call((), Some(dict))
        .expect("SimpleNamespace creation should succeed")
}

/// Create a frame dict with locals for testing [`crate::traceback_frames::extract_locals_dict`].
pub fn create_frame_dict_with_locals<'py>(
    py: Python<'py>,
    locals_dict: &Bound<'py, PyDict>,
) -> Bound<'py, PyDict> {
    let frame_dict = PyDict::new(py);
    frame_dict
        .set_item("filename", "test.py")
        .expect("set filename should succeed");
    frame_dict
        .set_item("lineno", 1)
        .expect("set lineno should succeed");
    frame_dict
        .set_item("name", "func")
        .expect("set name should succeed");
    frame_dict
        .set_item("locals", locals_dict)
        .expect("set locals should succeed");
    frame_dict
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
    pub fn build<'py>(self, py: Python<'py>) -> Bound<'py, PyAny> {
        let dict = PyDict::new(py);
        dict.set_item("filename", &self.filename)
            .expect("set filename should succeed");
        dict.set_item("lineno", self.lineno)
            .expect("set lineno should succeed");
        dict.set_item("name", &self.name)
            .expect("set name should succeed");

        if let Some(v) = self.end_lineno {
            dict.set_item("end_lineno", v)
                .expect("set end_lineno should succeed");
        }
        if let Some(v) = self.colno {
            dict.set_item("colno", v).expect("set colno should succeed");
        }
        if let Some(v) = self.end_colno {
            dict.set_item("end_colno", v)
                .expect("set end_colno should succeed");
        }
        if let Some(v) = &self.line {
            dict.set_item("line", v).expect("set line should succeed");
        }
        if let Some(entries) = &self.locals {
            let locals_dict = PyDict::new(py);
            for (k, v) in entries {
                locals_dict
                    .set_item(k, v)
                    .expect("set local entry should succeed");
            }
            dict.set_item("locals", locals_dict)
                .expect("set locals should succeed");
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
pub fn create_bad_repr_object<'py>(py: Python<'py>) -> Bound<'py, PyAny> {
    let globals = PyDict::new(py);
    py.run(
        c"class BadRepr:\n    def __repr__(self): raise ValueError('boom')",
        Some(&globals),
        None,
    )
    .expect("class definition should succeed");
    py.eval(c"BadRepr()", Some(&globals), None)
        .expect("object creation should succeed")
}

/// Assert that extracting a frame from the provided dict fails with an error
/// containing the expected substring.
pub fn assert_frame_extraction_error_contains(dict: &Bound<'_, PyDict>, expected_substr: &str) {
    let py = dict.py();
    let frame = create_simple_namespace(py, dict);
    let list = PyList::new(py, &[frame]).expect("list creation should succeed");
    let result = extract_frames_from_stack_summary(list.as_any());
    let err = result.expect_err("frame extraction should fail");
    let err_text = err.to_string();

    assert!(
        err_text.contains(expected_substr),
        "expected error containing {expected_substr:?}, got {err_text:?}"
    );
}
