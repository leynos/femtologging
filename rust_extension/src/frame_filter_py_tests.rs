//! Tests for Python frame filter bindings.

use super::*;
use pyo3::types::{PyDict, PyList};
use rstest::rstest;
use serial_test::serial;

fn make_stack_payload_dict<'py>(py: Python<'py>, filenames: &[&str]) -> Bound<'py, PyDict> {
    let frames = PyList::empty(py);
    for (i, filename) in filenames.iter().enumerate() {
        let frame = PyDict::new(py);
        frame
            .set_item("filename", *filename)
            .expect("failed to set frame filename");
        frame
            .set_item("lineno", i as u32 + 1)
            .expect("failed to set frame lineno");
        frame
            .set_item("function", format!("func_{i}"))
            .expect("failed to set frame function");
        frames.append(frame).expect("failed to append frame");
    }
    let payload = PyDict::new(py);
    payload
        .set_item("schema_version", 1u16)
        .expect("failed to set schema_version");
    payload
        .set_item("frames", frames)
        .expect("failed to set frames");
    payload
}

fn make_exception_payload_dict<'py>(py: Python<'py>, filenames: &[&str]) -> Bound<'py, PyDict> {
    let payload = make_stack_payload_dict(py, filenames);
    payload
        .set_item("type_name", "ValueError")
        .expect("failed to set type_name");
    payload
        .set_item("message", "test error")
        .expect("failed to set message");
    payload
}

/// Helper to call filter_frames and extract the resulting frames list.
fn filter_and_extract_frames<'py>(
    py: Python<'py>,
    payload: &Bound<'py, PyDict>,
    exclude_filenames: Option<Vec<String>>,
    exclude_functions: Option<Vec<String>>,
    max_depth: Option<usize>,
    exclude_logging: bool,
) -> Bound<'py, PyList> {
    let result = filter_frames(
        py,
        payload,
        exclude_filenames,
        exclude_functions,
        max_depth,
        exclude_logging,
    )
    .expect("filter_frames failed");
    let result_dict = result
        .downcast_bound::<PyDict>(py)
        .expect("result is not a dict");
    let frames = result_dict
        .get_item("frames")
        .expect("failed to get frames key")
        .expect("frames key is None");
    frames
        .downcast::<PyList>()
        .expect("frames is not a list")
        .clone()
}

#[rstest]
#[serial]
fn filter_stack_payload_exclude_logging() {
    Python::with_gil(|py| {
        let payload = make_stack_payload_dict(
            py,
            &[
                "myapp/main.py",
                "femtologging/__init__.py",
                "logging/__init__.py",
            ],
        );

        let frames_list = filter_and_extract_frames(py, &payload, None, None, None, true);

        assert_eq!(frames_list.len(), 1);
        let frame = frames_list.get_item(0).expect("failed to get first frame");
        let frame_dict = frame.downcast::<PyDict>().expect("frame is not a dict");
        let filename: String = frame_dict
            .get_item("filename")
            .expect("failed to get filename key")
            .expect("filename key is None")
            .extract()
            .expect("failed to extract filename");
        assert_eq!(filename, "myapp/main.py");
    });
}

#[rstest]
#[serial]
fn filter_stack_payload_exclude_filenames() {
    Python::with_gil(|py| {
        let payload = make_stack_payload_dict(
            py,
            &["myapp/main.py", ".venv/lib/requests.py", "myapp/utils.py"],
        );

        let frames_list = filter_and_extract_frames(
            py,
            &payload,
            Some(vec![".venv/".to_string()]),
            None,
            None,
            false,
        );

        assert_eq!(frames_list.len(), 2);
    });
}

#[rstest]
#[serial]
fn filter_stack_payload_max_depth() {
    Python::with_gil(|py| {
        let payload = make_stack_payload_dict(py, &["a.py", "b.py", "c.py", "d.py", "e.py"]);

        let frames_list = filter_and_extract_frames(py, &payload, None, None, Some(2), false);

        assert_eq!(frames_list.len(), 2);
        // Should be the last 2 frames (d.py, e.py)
        let frame0 = frames_list.get_item(0).expect("failed to get first frame");
        let frame0_dict = frame0.downcast::<PyDict>().expect("frame is not a dict");
        let filename0: String = frame0_dict
            .get_item("filename")
            .expect("failed to get filename key")
            .expect("filename key is None")
            .extract()
            .expect("failed to extract filename");
        assert_eq!(filename0, "d.py");
    });
}

#[rstest]
#[serial]
fn filter_exception_payload_detects_type() {
    Python::with_gil(|py| {
        let payload =
            make_exception_payload_dict(py, &["myapp/main.py", "femtologging/__init__.py"]);

        assert!(is_exception_payload(&payload).expect("is_exception_payload failed"));

        let result =
            filter_frames(py, &payload, None, None, None, true).expect("filter_frames failed");
        let result_dict = result
            .downcast_bound::<PyDict>(py)
            .expect("result is not a dict");

        // Should preserve exception fields
        let type_name: String = result_dict
            .get_item("type_name")
            .expect("failed to get type_name key")
            .expect("type_name key is None")
            .extract()
            .expect("failed to extract type_name");
        assert_eq!(type_name, "ValueError");

        let frames = result_dict
            .get_item("frames")
            .expect("failed to get frames key")
            .expect("frames key is None");
        let frames_list = frames.downcast::<PyList>().expect("frames is not a list");
        assert_eq!(frames_list.len(), 1);
    });
}

#[rstest]
#[serial]
fn filter_exception_payload_with_cause() {
    Python::with_gil(|py| {
        let cause = make_exception_payload_dict(py, &["cause.py", "femtologging/__init__.py"]);
        cause
            .set_item("type_name", "IOError")
            .expect("failed to set type_name");
        cause
            .set_item("message", "cause error")
            .expect("failed to set message");

        let payload = make_exception_payload_dict(py, &["main.py", "logging/__init__.py"]);
        payload
            .set_item("cause", cause)
            .expect("failed to set cause");

        let result =
            filter_frames(py, &payload, None, None, None, true).expect("filter_frames failed");
        let result_dict = result
            .downcast_bound::<PyDict>(py)
            .expect("result is not a dict");

        // Check main frames filtered
        let frames = result_dict
            .get_item("frames")
            .expect("failed to get frames key")
            .expect("frames key is None");
        let frames_list = frames.downcast::<PyList>().expect("frames is not a list");
        assert_eq!(frames_list.len(), 1);

        // Check cause frames also filtered
        let cause_result = result_dict
            .get_item("cause")
            .expect("failed to get cause key")
            .expect("cause key is None");
        let cause_dict = cause_result
            .downcast::<PyDict>()
            .expect("cause is not a dict");
        let cause_frames = cause_dict
            .get_item("frames")
            .expect("failed to get cause frames key")
            .expect("cause frames key is None");
        let cause_frames_list = cause_frames
            .downcast::<PyList>()
            .expect("cause frames is not a list");
        assert_eq!(cause_frames_list.len(), 1);
    });
}

#[rstest]
#[serial]
fn filter_stack_payload_exclude_functions() {
    Python::with_gil(|py| {
        let payload = make_stack_payload_dict(py, &["a.py", "b.py", "c.py"]);

        // Set function name on the second frame
        let frames = payload
            .get_item("frames")
            .expect("failed to get frames")
            .expect("frames is None");
        let frames_list = frames.downcast::<PyList>().expect("frames is not a list");
        let frame1 = frames_list.get_item(1).expect("failed to get frame 1");
        let frame1_dict = frame1.downcast::<PyDict>().expect("frame is not a dict");
        frame1_dict
            .set_item("function", "_internal_helper")
            .expect("failed to set function");

        let frames_list = filter_and_extract_frames(
            py,
            &payload,
            None,
            Some(vec!["_internal".to_string()]),
            None,
            false,
        );

        assert_eq!(frames_list.len(), 2);
    });
}

#[rstest]
#[serial]
fn filter_exception_payload_exclude_functions() {
    Python::with_gil(|py| {
        let payload = make_exception_payload_dict(py, &["a.py", "b.py", "c.py"]);

        // Set function name on the second frame
        let frames = payload
            .get_item("frames")
            .expect("failed to get frames")
            .expect("frames is None");
        let frames_list = frames.downcast::<PyList>().expect("frames is not a list");
        let frame1 = frames_list.get_item(1).expect("failed to get frame 1");
        let frame1_dict = frame1.downcast::<PyDict>().expect("frame is not a dict");
        frame1_dict
            .set_item("function", "_internal_helper")
            .expect("failed to set function");

        let result = filter_frames(
            py,
            &payload,
            None,
            Some(vec!["_internal".to_string()]),
            None,
            false,
        )
        .expect("filter_frames failed");
        let result_dict = result
            .downcast_bound::<PyDict>(py)
            .expect("result is not a dict");

        // Should preserve exception fields
        let type_name: String = result_dict
            .get_item("type_name")
            .expect("failed to get type_name key")
            .expect("type_name key is None")
            .extract()
            .expect("failed to extract type_name");
        assert_eq!(type_name, "ValueError");

        let frames = result_dict
            .get_item("frames")
            .expect("failed to get frames key")
            .expect("frames key is None");
        let frames_result = frames.downcast::<PyList>().expect("frames is not a list");
        assert_eq!(frames_result.len(), 2);
    });
}

#[rstest]
#[serial]
fn filter_frames_not_list_raises_type_error() {
    Python::with_gil(|py| {
        let payload = PyDict::new(py);
        payload
            .set_item("schema_version", 1u16)
            .expect("failed to set schema_version");
        payload
            .set_item("frames", "not a list")
            .expect("failed to set frames");

        let result = filter_frames(py, &payload, None, None, None, false);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("must be a list"));
    });
}

#[rstest]
#[serial]
fn filter_frame_not_dict_raises_type_error() {
    Python::with_gil(|py| {
        let frames = PyList::empty(py);
        frames.append("not a dict").expect("failed to append");

        let payload = PyDict::new(py);
        payload
            .set_item("schema_version", 1u16)
            .expect("failed to set schema_version");
        payload
            .set_item("frames", frames)
            .expect("failed to set frames");

        let result = filter_frames(py, &payload, None, None, None, false);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("must be a dict"));
    });
}

#[rstest]
#[serial]
fn get_logging_patterns_returns_expected() {
    let patterns = get_logging_infrastructure_patterns();
    assert!(patterns.contains(&"femtologging"));
    assert!(patterns.contains(&"logging/__init__"));
}

#[rstest]
#[serial]
fn filter_exception_payload_preserves_extra_keys() {
    Python::with_gil(|py| {
        let payload = make_exception_payload_dict(py, &["main.py"]);
        payload
            .set_item("custom_field", "preserved_value")
            .expect("failed to set custom_field");
        payload
            .set_item("thread_id", 12345)
            .expect("failed to set thread_id");

        let result =
            filter_frames(py, &payload, None, None, None, false).expect("filter_frames failed");
        let result_dict = result
            .downcast_bound::<PyDict>(py)
            .expect("result is not a dict");

        // Check custom fields are preserved
        let custom_field: String = result_dict
            .get_item("custom_field")
            .expect("failed to get custom_field key")
            .expect("custom_field key is None")
            .extract()
            .expect("failed to extract custom_field");
        assert_eq!(custom_field, "preserved_value");

        let thread_id: i32 = result_dict
            .get_item("thread_id")
            .expect("failed to get thread_id key")
            .expect("thread_id key is None")
            .extract()
            .expect("failed to extract thread_id");
        assert_eq!(thread_id, 12345);
    });
}
