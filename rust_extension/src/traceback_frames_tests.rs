//! Unit tests for stack frame extraction utilities.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use rstest::rstest;

use crate::traceback_frames::{extract_frames_from_stack_summary, extract_locals_dict};

// --------------------------------
// Test helpers
// --------------------------------

/// Create a SimpleNamespace object from a PyDict.
fn create_simple_namespace<'py>(py: Python<'py>, dict: &Bound<'py, PyDict>) -> Bound<'py, PyAny> {
    let types = py.import("types").expect("types module should exist");
    types
        .getattr("SimpleNamespace")
        .expect("SimpleNamespace should exist")
        .call((), Some(dict))
        .expect("SimpleNamespace creation should succeed")
}

/// Create a frame dict with locals for testing extract_locals_dict.
fn create_frame_dict_with_locals<'py>(
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

/// Test that a frame dict with a specific issue returns an error.
fn assert_frame_extraction_fails(dict: &Bound<'_, PyDict>, expected_msg: &str) {
    let py = dict.py();
    let frame = create_simple_namespace(py, dict);
    let list = PyList::new(py, &[frame]).expect("list creation should succeed");
    let result = extract_frames_from_stack_summary(list.as_any());
    assert!(result.is_err(), "{}", expected_msg);
}

/// Create a mock FrameSummary-like object with the given attributes.
fn create_mock_frame<'py>(
    py: Python<'py>,
    filename: &str,
    lineno: u32,
    name: &str,
    end_lineno: Option<u32>,
    colno: Option<u32>,
    end_colno: Option<u32>,
    line: Option<&str>,
    locals: Option<&[(&str, &str)]>,
) -> Bound<'py, PyAny> {
    let dict = PyDict::new(py);
    dict.set_item("filename", filename)
        .expect("set filename should succeed");
    dict.set_item("lineno", lineno)
        .expect("set lineno should succeed");
    dict.set_item("name", name)
        .expect("set name should succeed");

    if let Some(v) = end_lineno {
        dict.set_item("end_lineno", v)
            .expect("set end_lineno should succeed");
    }
    if let Some(v) = colno {
        dict.set_item("colno", v).expect("set colno should succeed");
    }
    if let Some(v) = end_colno {
        dict.set_item("end_colno", v)
            .expect("set end_colno should succeed");
    }
    if let Some(v) = line {
        dict.set_item("line", v).expect("set line should succeed");
    }
    if let Some(entries) = locals {
        let locals_dict = PyDict::new(py);
        for (k, v) in entries {
            locals_dict
                .set_item(*k, *v)
                .expect("set local entry should succeed");
        }
        dict.set_item("locals", locals_dict)
            .expect("set locals should succeed");
    }

    create_simple_namespace(py, &dict)
}

#[test]
fn frame_with_all_optional_fields_present() {
    Python::with_gil(|py| {
        let frame = create_mock_frame(
            py,
            "test.py",
            42,
            "test_function",
            Some(45),
            Some(4),
            Some(20),
            Some("    result = compute(x)"),
            Some(&[("x", "10"), ("result", "None")]),
        );

        let list = PyList::new(py, &[frame]).expect("list creation should succeed");
        let frames =
            extract_frames_from_stack_summary(list.as_any()).expect("extraction should succeed");

        assert_eq!(frames.len(), 1);
        let result = &frames[0];
        assert_eq!(result.filename, "test.py");
        assert_eq!(result.lineno, 42);
        assert_eq!(result.function, "test_function");
        assert_eq!(result.end_lineno, Some(45));
        assert_eq!(result.colno, Some(4));
        assert_eq!(result.end_colno, Some(20));
        assert_eq!(
            result.source_line,
            Some("    result = compute(x)".to_string())
        );
        let locals = result.locals.as_ref().expect("locals should be present");
        assert_eq!(locals.get("x"), Some(&"'10'".to_string()));
        assert_eq!(locals.get("result"), Some(&"'None'".to_string()));
    });
}

#[rstest]
#[case::no_optional_fields(None, None, None, None, None)]
#[case::only_end_lineno(Some(50), None, None, None, None)]
#[case::only_colno(None, Some(8), None, None, None)]
#[case::only_source_line(None, None, None, Some("x = 1"), None)]
fn frame_with_missing_optional_fields(
    #[case] end_lineno: Option<u32>,
    #[case] colno: Option<u32>,
    #[case] end_colno: Option<u32>,
    #[case] line: Option<&str>,
    #[case] locals: Option<&[(&str, &str)]>,
) {
    Python::with_gil(|py| {
        let frame = create_mock_frame(
            py,
            "module.py",
            10,
            "my_func",
            end_lineno,
            colno,
            end_colno,
            line,
            locals,
        );

        let list = PyList::new(py, &[frame]).expect("list creation should succeed");
        let frames =
            extract_frames_from_stack_summary(list.as_any()).expect("extraction should succeed");

        assert_eq!(frames.len(), 1);
        let result = &frames[0];
        assert_eq!(result.filename, "module.py");
        assert_eq!(result.lineno, 10);
        assert_eq!(result.function, "my_func");
        assert_eq!(result.end_lineno, end_lineno);
        assert_eq!(result.colno, colno);
        assert_eq!(result.end_colno, end_colno);
        assert_eq!(result.source_line, line.map(String::from));
        assert_eq!(result.locals, None);
    });
}

/// Which required field is missing from the frame dict.
#[derive(Debug)]
enum MissingField {
    Filename,
    Lineno,
}

#[rstest]
#[case::missing_filename(MissingField::Filename, "should fail when filename is missing")]
#[case::missing_lineno(MissingField::Lineno, "should fail when lineno is missing")]
fn frame_missing_required_field_returns_error(
    #[case] missing: MissingField,
    #[case] expected_msg: &str,
) {
    Python::with_gil(|py| {
        let dict = PyDict::new(py);
        match missing {
            MissingField::Filename => {
                dict.set_item("lineno", 1)
                    .expect("set lineno should succeed");
            }
            MissingField::Lineno => {
                dict.set_item("filename", "test.py")
                    .expect("set filename should succeed");
            }
        }
        dict.set_item("name", "func")
            .expect("set name should succeed");

        assert_frame_extraction_fails(&dict, expected_msg);
    });
}

#[test]
fn frame_with_wrong_type_lineno_returns_error() {
    Python::with_gil(|py| {
        let dict = PyDict::new(py);
        dict.set_item("filename", "test.py")
            .expect("set filename should succeed");
        dict.set_item("lineno", "not_a_number")
            .expect("set lineno should succeed");
        dict.set_item("name", "func")
            .expect("set name should succeed");

        assert_frame_extraction_fails(&dict, "should fail when lineno has wrong type");
    });
}

/// Key descriptor for parameterized locals extraction tests.
/// Keys starting with digits are parsed as integers; others are strings.
#[derive(Debug, Clone)]
struct LocalEntry {
    key: &'static str,
    value: &'static str,
}

impl LocalEntry {
    const fn new(key: &'static str, value: &'static str) -> Self {
        Self { key, value }
    }

    /// Returns true if the key should be inserted as an integer.
    fn is_int_key(&self) -> bool {
        self.key.chars().next().is_some_and(|c| c.is_ascii_digit())
    }
}

#[rstest]
#[case::mixed_valid_and_invalid(
    &[LocalEntry::new("valid_key", "valid_value"), LocalEntry::new("123", "int_key_value")],
    Some(1),
    "should return partial locals when some entries fail"
)]
#[case::all_invalid_int_keys(
    &[LocalEntry::new("1", "value1"), LocalEntry::new("2", "value2")],
    None,
    "should return None when all entries fail"
)]
fn extract_locals_handles_mixed_entries(
    #[case] entries: &[LocalEntry],
    #[case] expected: Option<usize>,
    #[case] description: &str,
) {
    Python::with_gil(|py| {
        let locals_dict = PyDict::new(py);
        for entry in entries {
            if entry.is_int_key() {
                let int_key: i32 = entry.key.parse().expect("int key should parse");
                locals_dict
                    .set_item(int_key, entry.value)
                    .expect("set int key entry should succeed");
            } else {
                locals_dict
                    .set_item(entry.key, entry.value)
                    .expect("set string key entry should succeed");
            }
        }

        let frame_dict = create_frame_dict_with_locals(py, &locals_dict);
        let frame = create_simple_namespace(py, &frame_dict);

        let result = extract_locals_dict(&frame);
        match expected {
            Some(count) => {
                let locals = result.expect(description);
                assert_eq!(locals.len(), count, "{}", description);
                if count == 1 {
                    assert_eq!(
                        locals.get("valid_key"),
                        Some(&"'valid_value'".to_string()),
                        "valid_key should have expected value"
                    );
                }
            }
            None => {
                assert!(result.is_none(), "{}", description);
            }
        }
    });
}

#[test]
fn extract_frames_from_stack_summary_converts_list() {
    Python::with_gil(|py| {
        let frame1 = create_mock_frame(py, "a.py", 1, "func_a", None, None, None, None, None);
        let frame2 = create_mock_frame(py, "b.py", 2, "func_b", None, None, None, None, None);

        let list = PyList::new(py, &[frame1, frame2]).expect("list creation should succeed");

        let frames =
            extract_frames_from_stack_summary(list.as_any()).expect("extraction should succeed");

        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].filename, "a.py");
        assert_eq!(frames[0].function, "func_a");
        assert_eq!(frames[1].filename, "b.py");
        assert_eq!(frames[1].function, "func_b");
    });
}

#[test]
fn extract_frames_from_stack_summary_empty_list() {
    Python::with_gil(|py| {
        let list = PyList::empty(py);

        let frames =
            extract_frames_from_stack_summary(list.as_any()).expect("extraction should succeed");

        assert!(frames.is_empty());
    });
}

#[test]
fn extract_frames_from_stack_summary_not_a_list_returns_error() {
    Python::with_gil(|py| {
        let dict = PyDict::new(py);

        let result = extract_frames_from_stack_summary(dict.as_any());
        assert!(result.is_err(), "should fail when input is not a list");
    });
}
