//! Unit tests for stack frame extraction utilities.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use rstest::rstest;

use crate::test_utils::traceback_test_helpers::*;
use crate::traceback_frames::{extract_frames_from_stack_summary, extract_locals_dict};

#[test]
fn frame_with_all_optional_fields_present() {
    Python::with_gil(|py| {
        let frame = MockFrameBuilder::new("test.py", 42, "test_function")
            .end_lineno(45)
            .colno(4)
            .end_colno(20)
            .line("    result = compute(x)")
            .locals(&[("x", "10"), ("result", "None")])
            .build(py);

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
#[case::no_optional_fields(None, None, None, None)]
#[case::only_end_lineno(Some(50), None, None, None)]
#[case::only_colno(None, Some(8), None, None)]
#[case::only_source_line(None, None, None, Some("x = 1"))]
fn frame_with_missing_optional_fields(
    #[case] end_lineno: Option<u32>,
    #[case] colno: Option<u32>,
    #[case] end_colno: Option<u32>,
    #[case] line: Option<&str>,
) {
    Python::with_gil(|py| {
        let mut builder = MockFrameBuilder::new("module.py", 10, "my_func");
        if let Some(v) = end_lineno {
            builder = builder.end_lineno(v);
        }
        if let Some(v) = colno {
            builder = builder.colno(v);
        }
        if let Some(v) = end_colno {
            builder = builder.end_colno(v);
        }
        if let Some(v) = line {
            builder = builder.line(v);
        }
        let frame = builder.build(py);

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
    Name,
}

#[rstest]
#[case::missing_filename(MissingField::Filename, "filename")]
#[case::missing_lineno(MissingField::Lineno, "lineno")]
#[case::missing_name(MissingField::Name, "name")]
fn frame_missing_required_field_returns_error(
    #[case] missing: MissingField,
    #[case] expected_error_substr: &str,
) {
    Python::with_gil(|py| {
        let dict = PyDict::new(py);
        match missing {
            MissingField::Filename => {
                dict.set_item("lineno", 1)
                    .expect("set lineno should succeed");
                dict.set_item("name", "func")
                    .expect("set name should succeed");
            }
            MissingField::Lineno => {
                dict.set_item("filename", "test.py")
                    .expect("set filename should succeed");
                dict.set_item("name", "func")
                    .expect("set name should succeed");
            }
            MissingField::Name => {
                dict.set_item("filename", "test.py")
                    .expect("set filename should succeed");
                dict.set_item("lineno", 1)
                    .expect("set lineno should succeed");
            }
        }

        assert_frame_extraction_error_contains(&dict, expected_error_substr);
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

        assert_frame_extraction_error_contains(&dict, "integer");
    });
}

#[rstest]
#[case::mixed_valid_and_invalid(
    &[LocalEntry::new("valid_key", "valid_value"), LocalEntry::new("123", "int_key_value")],
    Some(&[("valid_key", "'valid_value'")] as &[_]),
    "should return partial locals when some entries fail"
)]
#[case::all_invalid_int_keys(
    &[LocalEntry::new("1", "value1"), LocalEntry::new("2", "value2")],
    None,
    "should return None when all entries fail"
)]
fn extract_locals_handles_mixed_entries(
    #[case] entries: &[LocalEntry],
    #[case] expected: Option<&[(&str, &str)]>,
    #[case] description: &str,
) {
    Python::with_gil(|py| {
        let locals_dict = PyDict::new(py);
        populate_locals_dict_from_entries(&locals_dict, entries);
        assert_locals_extraction_result(&locals_dict, expected, description);
    });
}

#[test]
fn extract_frames_from_stack_summary_converts_list() {
    Python::with_gil(|py| {
        let frame1 = MockFrameBuilder::new("a.py", 1, "func_a").build(py);
        let frame2 = MockFrameBuilder::new("b.py", 2, "func_b").build(py);

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

// --------------------------------
// Skip path behaviour tests
// --------------------------------

#[rstest]
#[case::mixed_skip_reasons(
    &[
        LocalEntry::new("valid", "value"),
        LocalEntry::new("123", "int_key"),
        LocalEntry::new("456", "another_int"),
    ],
    Some(&[("valid", "'value'")] as &[_]),
    "should handle multiple non-string keys without panic"
)]
#[case::many_invalid_entries(
    &[
        LocalEntry::new("1", "a"),
        LocalEntry::new("2", "b"),
        LocalEntry::new("3", "c"),
        LocalEntry::new("4", "d"),
        LocalEntry::new("5", "e"),
    ],
    None,
    "should handle many invalid entries without panic"
)]
#[case::single_valid_among_many_invalid(
    &[
        LocalEntry::new("1", "a"),
        LocalEntry::new("ok", "value"),
        LocalEntry::new("2", "b"),
        LocalEntry::new("3", "c"),
    ],
    Some(&[("ok", "'value'")] as &[_]),
    "should extract single valid entry among many invalid"
)]
fn extract_locals_skip_path_does_not_panic(
    #[case] entries: &[LocalEntry],
    #[case] expected: Option<&[(&str, &str)]>,
    #[case] description: &str,
) {
    Python::with_gil(|py| {
        let locals_dict = PyDict::new(py);
        populate_locals_dict_from_entries(&locals_dict, entries);
        assert_locals_extraction_result(&locals_dict, expected, description);
    });
}

/// Scenario for testing locals extraction with various skip reasons.
#[derive(Debug)]
enum SkipScenario {
    /// Single valid entry, one bad repr object.
    ReprFailure,
    /// Single valid entry, one integer key, one bad repr object.
    MixedSkipReasons,
    /// All entries have failing `__repr__` or non-string keys.
    AllReprFailures,
}

#[rstest]
#[case::repr_failure(SkipScenario::ReprFailure, Some(("good", "'value'")))]
#[case::mixed_skip_reasons(SkipScenario::MixedSkipReasons, Some(("valid", "'value'")))]
#[case::all_repr_failures(SkipScenario::AllReprFailures, None)]
fn extract_locals_with_skip_reasons_returns_partial(
    #[case] scenario: SkipScenario,
    #[case] expected: Option<(&str, &str)>,
) {
    Python::with_gil(|py| {
        let locals_dict = PyDict::new(py);

        // Add a valid entry only for scenarios that expect one
        if let Some((key, _)) = expected {
            locals_dict
                .set_item(key, "value")
                .expect("set valid entry should succeed");
        }

        // Add scenario-specific invalid entries
        match scenario {
            SkipScenario::ReprFailure => {
                let bad_repr_obj = create_bad_repr_object(py);
                locals_dict
                    .set_item("bad", bad_repr_obj)
                    .expect("set bad repr entry should succeed");
            }
            SkipScenario::MixedSkipReasons => {
                locals_dict
                    .set_item(42, "int_key_value")
                    .expect("set int key entry should succeed");

                let bad_repr_obj = create_bad_repr_object(py);
                locals_dict
                    .set_item("bad_repr", bad_repr_obj)
                    .expect("set bad repr entry should succeed");
            }
            SkipScenario::AllReprFailures => {
                // Add multiple entries with failing repr
                for name in ["bad1", "bad2", "bad3"] {
                    let bad_repr_obj = create_bad_repr_object(py);
                    locals_dict
                        .set_item(name, bad_repr_obj)
                        .expect("set bad repr entry should succeed");
                }
                // Also add a non-string key
                locals_dict
                    .set_item(99, "int_key_value")
                    .expect("set int key entry should succeed");
            }
        }

        let frame_dict = create_frame_dict_with_locals(py, &locals_dict);
        let frame = create_simple_namespace(py, &frame_dict);

        let result = extract_locals_dict(&frame);

        match expected {
            Some((key, value)) => {
                let locals = result.expect("should return partial locals");
                assert_eq!(locals.len(), 1, "should have exactly one entry");
                assert_eq!(
                    locals.get(key),
                    Some(&value.to_string()),
                    "should have the valid entry with repr'd value"
                );
            }
            None => {
                assert!(result.is_none(), "should return None when all entries fail");
            }
        }
    });
}
