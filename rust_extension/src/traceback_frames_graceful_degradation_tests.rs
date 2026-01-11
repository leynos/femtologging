//! Graceful degradation tests for stack frame extraction utilities.
//!
//! These tests verify that frame extraction handles edge cases gracefully,
//! such as wrong types for optional fields, non-mapping locals, and non-string keys.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::test_utils::traceback_test_helpers::*;
use crate::traceback_frames::{extract_frames_from_stack_summary, extract_locals_dict};

#[test]
fn frame_with_wrong_type_optional_field_degrades_to_none() {
    // Optional fields with wrong types should degrade to None, not error.
    // This tests the graceful degradation behaviour of get_optional_attr.
    Python::with_gil(|py| {
        let dict = PyDict::new(py);
        dict.set_item("filename", "test.py")
            .expect("set filename should succeed");
        dict.set_item("lineno", 42)
            .expect("set lineno should succeed");
        dict.set_item("name", "test_func")
            .expect("set name should succeed");
        // end_lineno should be u32, but we provide a string
        dict.set_item("end_lineno", "not_a_number")
            .expect("set end_lineno should succeed");
        // colno should be u32, but we provide a list
        dict.set_item("colno", PyList::empty(py))
            .expect("set colno should succeed");

        let frame = create_simple_namespace(py, &dict);
        let list = PyList::new(py, &[frame]).expect("list creation should succeed");

        let frames =
            extract_frames_from_stack_summary(list.as_any()).expect("extraction should succeed");

        assert_eq!(frames.len(), 1);
        let result = &frames[0];
        assert_frame_required_fields(result, "test.py", 42, "test_func");
        // Wrong-type optional fields degrade to None
        assert_frame_optional_fields(result, ExpectedOptionalFields::default());
    });
}

#[test]
fn frame_with_explicit_none_optional_fields_degrades_to_none() {
    // Optional fields explicitly set to Python None should become Rust None.
    Python::with_gil(|py| {
        let dict = PyDict::new(py);
        dict.set_item("filename", "test.py")
            .expect("set filename should succeed");
        dict.set_item("lineno", 10)
            .expect("set lineno should succeed");
        dict.set_item("name", "my_func")
            .expect("set name should succeed");
        // Explicitly set optional fields to None
        dict.set_item("end_lineno", py.None())
            .expect("set end_lineno should succeed");
        dict.set_item("colno", py.None())
            .expect("set colno should succeed");
        dict.set_item("end_colno", py.None())
            .expect("set end_colno should succeed");
        dict.set_item("line", py.None())
            .expect("set line should succeed");
        dict.set_item("locals", py.None())
            .expect("set locals should succeed");

        let frame = create_simple_namespace(py, &dict);
        let list = PyList::new(py, &[frame]).expect("list creation should succeed");

        let frames =
            extract_frames_from_stack_summary(list.as_any()).expect("extraction should succeed");

        assert_eq!(frames.len(), 1);
        let result = &frames[0];
        assert_frame_required_fields(result, "test.py", 10, "my_func");
        // All optional fields should be None
        assert_frame_optional_fields(result, ExpectedOptionalFields::default());
        assert_eq!(result.locals, None);
    });
}

#[test]
fn extract_locals_with_unrepr_able_value_skips_entry() {
    // Values that raise on repr() should be skipped, preserving other entries.
    Python::with_gil(|py| {
        // Create a class whose __repr__ raises an exception
        let code = c"
class UnreprAble:
    def __repr__(self):
        raise RuntimeError('cannot repr')
unrepr_obj = UnreprAble()
";
        let globals = PyDict::new(py);
        py.run(code, Some(&globals), None)
            .expect("code to create unrepr-able object should succeed");
        let unrepr_obj = globals
            .get_item("unrepr_obj")
            .expect("get_item should not fail")
            .expect("unrepr_obj should exist");

        let locals_dict = PyDict::new(py);
        locals_dict
            .set_item("good_key", "good_value")
            .expect("set good_key should succeed");
        locals_dict
            .set_item("bad_key", unrepr_obj)
            .expect("set bad_key should succeed");
        locals_dict
            .set_item("another_good", 42)
            .expect("set another_good should succeed");

        let frame_dict = create_frame_dict_with_locals(py, &locals_dict);
        let frame = create_simple_namespace(py, &frame_dict);

        let result = extract_locals_dict(&frame);
        let locals = result.expect("should return partial locals");

        // bad_key should be skipped, but good entries preserved
        assert_eq!(locals.len(), 2);
        assert_local_equals(&locals, "good_key", "'good_value'");
        assert_local_equals(&locals, "another_good", "42");
        assert_local_absent(&locals, "bad_key", "unrepr-able entry should be skipped");
    });
}

#[test]
fn extract_locals_with_non_mapping_locals_degrades_gracefully() {
    // A locals attribute that is not a mapping (list instead of dict)
    // should degrade gracefully to None, not error.
    Python::with_gil(|py| {
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
        // Set locals to a list instead of a dict
        let non_mapping_locals = PyList::new(py, &[1, 2, 3]).expect("list creation should succeed");
        frame_dict
            .set_item("locals", non_mapping_locals)
            .expect("set locals should succeed");

        let frame = create_simple_namespace(py, &frame_dict);

        // extract_locals_dict should not panic or error; it should return None
        let result = extract_locals_dict(&frame);
        assert!(
            result.is_none(),
            "expected non-mapping locals to yield None from extract_locals_dict"
        );
    });
}

#[test]
fn extract_locals_with_non_string_keys_degrades_gracefully() {
    // A locals mapping that uses non-string keys should degrade gracefully,
    // skipping entries with invalid keys rather than erroring.
    Python::with_gil(|py| {
        let locals_dict = PyDict::new(py);
        // Add entries with non-string keys (integers)
        locals_dict
            .set_item(1, "value-for-int-key")
            .expect("set int key should succeed");
        locals_dict
            .set_item(2, "another-int-key-value")
            .expect("set another int key should succeed");

        let frame_dict = create_frame_dict_with_locals(py, &locals_dict);
        let frame = create_simple_namespace(py, &frame_dict);

        // extract_locals_dict should not error; non-string keys are skipped
        let result = extract_locals_dict(&frame);

        // When all keys are invalid, we get None (empty map degrades to None)
        assert!(
            result.is_none(),
            "expected locals with only non-string keys to yield None"
        );
    });
}
