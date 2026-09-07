//! Unit tests for stack frame extraction utilities.
//!
//! Graceful degradation tests (type mismatches, non-mapping locals, non-string
//! keys) are in [`crate::traceback_frames_graceful_degradation_tests`].

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use rstest::rstest;

use crate::test_utils::traceback_test_helpers::*;
use crate::traceback_frames::extract_frames_from_stack_summary;

#[test]
fn frame_with_all_optional_fields_present() {
    Python::attach(|py| {
        let frame = MockFrameBuilder::new("test.py", 42, "test_function")
            .end_lineno(45)
            .colno(4)
            .end_colno(20)
            .line("    result = compute(x)")
            .locals(&[("x", "10"), ("result", "None")])
            .build(py)
            .expect("mock frame should build");

        let list = PyList::new(py, &[frame]).expect("list creation should succeed");
        let frames =
            extract_frames_from_stack_summary(list.as_any()).expect("extraction should succeed");

        assert_eq!(frames.len(), 1);
        let [result] = frames.as_slice() else {
            panic!("exactly one frame should be extracted");
        };
        assert_eq!(result.filename, "test.py");
        assert_eq!(result.lineno, 42);
        assert_eq!(result.function, "test_function");
        assert_eq!(result.end_lineno, Some(45));
        assert_eq!(result.colno, Some(4));
        assert_eq!(result.end_colno, Some(20));
        assert_eq!(
            result.source_line,
            Some("    result = compute(x)".to_owned())
        );
        let locals = result.locals.as_ref().expect("locals should be present");
        assert_local_equals(locals, "x", "'10'");
        assert_local_equals(locals, "result", "'None'");
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
    Python::attach(|py| {
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
        let frame = builder.build(py).expect("mock frame should build");

        let list = PyList::new(py, &[frame]).expect("list creation should succeed");
        let frames =
            extract_frames_from_stack_summary(list.as_any()).expect("extraction should succeed");

        assert_eq!(frames.len(), 1);
        let [result] = frames.as_slice() else {
            panic!("exactly one frame should be extracted");
        };
        assert_frame_required_fields(result, "module.py", 10, "my_func");
        assert_frame_optional_fields(
            result,
            ExpectedOptionalFields {
                end_lineno,
                colno,
                end_colno,
                source_line: line,
            },
        );
        assert_eq!(result.locals, None);
    });
}

/// Which required field is missing from the frame dict.
#[derive(Debug, Clone, Copy)]
enum MissingField {
    Filename,
    Lineno,
    Name,
}

impl MissingField {
    /// The frame attribute name this variant omits.
    const fn field_name(self) -> &'static str {
        match self {
            Self::Filename => "filename",
            Self::Lineno => "lineno",
            Self::Name => "name",
        }
    }
}

#[rstest]
#[case::missing_filename(MissingField::Filename, "filename")]
#[case::missing_lineno(MissingField::Lineno, "lineno")]
#[case::missing_name(MissingField::Name, "name")]
fn frame_missing_required_field_returns_error(
    #[case] missing: MissingField,
    #[case] expected_error_substr: &str,
) {
    Python::attach(|py| {
        let dict = base_frame_dict(py).expect("base frame dict should build");
        dict.del_item(missing.field_name())
            .expect("removing the required field should succeed");

        assert_frame_extraction_error_contains(&dict, expected_error_substr)
            .expect("frame arrangement should succeed");
    });
}

#[test]
fn frame_with_wrong_type_lineno_returns_error() {
    Python::attach(|py| {
        let dict = base_frame_dict(py).expect("base frame dict should build");
        dict.set_item("lineno", "not_a_number")
            .expect("set lineno should succeed");

        assert_frame_extraction_error_contains(&dict, "integer")
            .expect("frame arrangement should succeed");
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
#[case::multiple_non_string_keys(
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
fn extract_locals_handles_mixed_entries(
    #[case] entries: &[LocalEntry],
    #[case] expected: Option<&[(&str, &str)]>,
    #[case] description: &str,
) {
    Python::attach(|py| {
        let locals_dict = PyDict::new(py);
        populate_locals_dict_from_entries(&locals_dict, entries)
            .expect("locals dict should populate");
        check_locals_extraction_result(&locals_dict, expected, description)
            .expect("extracted locals should match the expected entries");
    });
}

#[test]
fn extract_frames_from_stack_summary_converts_list() {
    Python::attach(|py| {
        let first_summary = MockFrameBuilder::new("a.py", 1, "func_a")
            .build(py)
            .expect("mock frame should build");
        let second_summary = MockFrameBuilder::new("b.py", 2, "func_b")
            .build(py)
            .expect("mock frame should build");

        let list = PyList::new(py, &[first_summary, second_summary])
            .expect("list creation should succeed");

        let frames =
            extract_frames_from_stack_summary(list.as_any()).expect("extraction should succeed");

        let [first_frame, second_frame] = frames.as_slice() else {
            panic!("expected two frames, received {}", frames.len());
        };
        assert_eq!(first_frame.filename, "a.py");
        assert_eq!(first_frame.function, "func_a");
        assert_eq!(second_frame.filename, "b.py");
        assert_eq!(second_frame.function, "func_b");
    });
}

#[test]
fn extract_frames_from_stack_summary_empty_list() {
    Python::attach(|py| {
        let list = PyList::empty(py);

        let frames =
            extract_frames_from_stack_summary(list.as_any()).expect("extraction should succeed");

        assert!(frames.is_empty());
    });
}

#[test]
fn extract_frames_from_stack_summary_not_a_list_returns_error() {
    Python::attach(|py| {
        let dict = PyDict::new(py);

        let result = extract_frames_from_stack_summary(dict.as_any());
        assert!(result.is_err(), "should fail when input is not a list");
    });
}

/// Scenario for testing locals extraction with various skip reasons.
#[derive(Debug)]
enum SkipScenario {
    /// Single valid entry, one bad repr object.
    ReprFailure,
    /// Single valid entry, one integer key, one bad repr object.
    MixedSkipReasons,
    /// All entries fail: some with failing `__repr__`, others with non-string keys.
    AllFailMixed,
}

#[rstest]
#[case::repr_failure(SkipScenario::ReprFailure, Some(("good", "'value'")))]
#[case::mixed_skip_reasons(SkipScenario::MixedSkipReasons, Some(("valid", "'value'")))]
#[case::all_fail_mixed(SkipScenario::AllFailMixed, None)]
fn extract_locals_with_skip_reasons_returns_partial(
    #[case] scenario: SkipScenario,
    #[case] expected: Option<(&str, &str)>,
) {
    Python::attach(|py| {
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
                add_bad_repr_entry(&locals_dict, "bad").expect("bad repr entry should insert");
            }
            SkipScenario::MixedSkipReasons => {
                locals_dict
                    .set_item(42, "int_key_value")
                    .expect("set int key entry should succeed");
                add_bad_repr_entry(&locals_dict, "bad_repr").expect("bad repr entry should insert");
            }
            SkipScenario::AllFailMixed => {
                // Add multiple entries with failing repr
                for name in ["bad1", "bad2", "bad3"] {
                    add_bad_repr_entry(&locals_dict, name).expect("bad repr entry should insert");
                }
                // Also add a non-string key
                locals_dict
                    .set_item(99, "int_key_value")
                    .expect("set int key entry should succeed");
            }
        }

        let expected_array = expected.map(|(key, value)| [(key, value)]);
        let expected_slice = expected_array.as_ref().map(<[(&str, &str); 1]>::as_slice);
        check_locals_extraction_result(
            &locals_dict,
            expected_slice,
            "should handle skip scenario correctly",
        )
        .expect("extracted locals should match the expected entries");
    });
}
