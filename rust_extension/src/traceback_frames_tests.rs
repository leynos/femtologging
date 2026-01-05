//! Unit tests for stack frame extraction utilities.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use rstest::rstest;

use crate::traceback_frames::{extract_frames_from_stack_summary, extract_locals_dict};

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

    // Create a SimpleNamespace-like object from the dict
    let types = py.import("types").expect("types module should exist");
    types
        .getattr("SimpleNamespace")
        .expect("SimpleNamespace should exist")
        .call((), Some(&dict))
        .expect("SimpleNamespace creation should succeed")
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

#[test]
fn frame_missing_required_filename_returns_error() {
    Python::with_gil(|py| {
        let dict = PyDict::new(py);
        dict.set_item("lineno", 1)
            .expect("set lineno should succeed");
        dict.set_item("name", "func")
            .expect("set name should succeed");
        // Missing filename

        let types = py.import("types").expect("types module should exist");
        let frame = types
            .getattr("SimpleNamespace")
            .expect("SimpleNamespace should exist")
            .call((), Some(&dict))
            .expect("SimpleNamespace creation should succeed");

        let list = PyList::new(py, &[frame]).expect("list creation should succeed");
        let result = extract_frames_from_stack_summary(list.as_any());
        assert!(result.is_err(), "should fail when filename is missing");
    });
}

#[test]
fn frame_missing_required_lineno_returns_error() {
    Python::with_gil(|py| {
        let dict = PyDict::new(py);
        dict.set_item("filename", "test.py")
            .expect("set filename should succeed");
        dict.set_item("name", "func")
            .expect("set name should succeed");
        // Missing lineno

        let types = py.import("types").expect("types module should exist");
        let frame = types
            .getattr("SimpleNamespace")
            .expect("SimpleNamespace should exist")
            .call((), Some(&dict))
            .expect("SimpleNamespace creation should succeed");

        let list = PyList::new(py, &[frame]).expect("list creation should succeed");
        let result = extract_frames_from_stack_summary(list.as_any());
        assert!(result.is_err(), "should fail when lineno is missing");
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

        let types = py.import("types").expect("types module should exist");
        let frame = types
            .getattr("SimpleNamespace")
            .expect("SimpleNamespace should exist")
            .call((), Some(&dict))
            .expect("SimpleNamespace creation should succeed");

        let list = PyList::new(py, &[frame]).expect("list creation should succeed");
        let result = extract_frames_from_stack_summary(list.as_any());
        assert!(result.is_err(), "should fail when lineno has wrong type");
    });
}

#[test]
fn extract_locals_skips_failed_entries() {
    Python::with_gil(|py| {
        // Create a dict with mixed valid and invalid entries
        let locals_dict = PyDict::new(py);
        locals_dict
            .set_item("valid_key", "valid_value")
            .expect("set valid entry should succeed");
        // Add an entry with a non-string key (integer)
        locals_dict
            .set_item(123, "int_key_value")
            .expect("set int key entry should succeed");

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

        let types = py.import("types").expect("types module should exist");
        let frame = types
            .getattr("SimpleNamespace")
            .expect("SimpleNamespace should exist")
            .call((), Some(&frame_dict))
            .expect("SimpleNamespace creation should succeed");

        let result = extract_locals_dict(&frame);
        let locals = result.expect("should return partial locals");
        assert_eq!(locals.len(), 1, "should have one valid entry");
        assert_eq!(locals.get("valid_key"), Some(&"'valid_value'".to_string()));
    });
}

#[test]
fn extract_locals_returns_none_when_all_fail() {
    Python::with_gil(|py| {
        // Create a dict with only invalid entries (non-string keys)
        let locals_dict = PyDict::new(py);
        locals_dict
            .set_item(1, "value1")
            .expect("set int key 1 should succeed");
        locals_dict
            .set_item(2, "value2")
            .expect("set int key 2 should succeed");

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

        let types = py.import("types").expect("types module should exist");
        let frame = types
            .getattr("SimpleNamespace")
            .expect("SimpleNamespace should exist")
            .call((), Some(&frame_dict))
            .expect("SimpleNamespace creation should succeed");

        let result = extract_locals_dict(&frame);
        assert!(result.is_none(), "should return None when all entries fail");
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
