//! Regression tests for compatibility of Python payload filtering.

use super::*;
use pyo3::types::PyDict;
use rstest::rstest;
use serial_test::serial;

#[rstest]
#[serial]
fn filter_exception_payload_preserves_extra_keys() {
    Python::attach(|py| {
        let payload = make_exception_payload_dict(py, &["main.py"]).expect("payload should build");
        payload
            .set_item("custom_field", "preserved_value")
            .expect("failed to set custom_field");
        payload
            .set_item("thread_id", 12345)
            .expect("failed to set thread_id");

        let result = filter_frames_with_defaults(py, &payload, TestFilterOptions::default())
            .expect("filter_frames failed");
        let result_dict = result
            .cast_bound::<PyDict>(py)
            .expect("result is not a dict");
        let custom_field: String = result_dict
            .get_item("custom_field")
            .expect("custom field lookup should succeed")
            .expect("custom field should remain")
            .extract()
            .expect("custom field should be a string");
        let thread_id: i32 = result_dict
            .get_item("thread_id")
            .expect("thread id lookup should succeed")
            .expect("thread id should remain")
            .extract()
            .expect("thread id should be an integer");

        assert_eq!(custom_field, "preserved_value");
        assert_eq!(thread_id, 12345);
    });
}
