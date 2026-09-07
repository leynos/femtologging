//! Tests for Python frame filter bindings.

use pyo3::types::{PyDict, PyList};
use rstest::rstest;
use serial_test::serial;

use super::*;

fn make_stack_payload_dict<'py>(
    py: Python<'py>,
    filenames: &[&str],
) -> PyResult<Bound<'py, PyDict>> {
    let frames = PyList::empty(py);
    for (i, filename) in filenames.iter().enumerate() {
        let frame = PyDict::new(py);
        frame.set_item("filename", *filename)?;
        let line_number = u32::try_from(i + 1)
            .map_err(|_| pyo3::exceptions::PyOverflowError::new_err("frame index exceeds u32"))?;
        frame.set_item("lineno", line_number)?;
        frame.set_item("function", format!("func_{i}"))?;
        frames.append(frame)?;
    }
    let payload = PyDict::new(py);
    payload.set_item("schema_version", 1u16)?;
    payload.set_item("frames", frames)?;
    Ok(payload)
}

fn make_exception_payload_dict<'py>(
    py: Python<'py>,
    filenames: &[&str],
) -> PyResult<Bound<'py, PyDict>> {
    let payload = make_stack_payload_dict(py, filenames)?;
    payload.set_item("type_name", "ValueError")?;
    payload.set_item("message", "test error")?;
    Ok(payload)
}

/// Build filtering options for a direct Rust filtering test.
fn filter_options(
    exclude_filenames: Option<Vec<String>>,
    exclude_functions: Option<Vec<String>>,
    max_depth: Option<usize>,
    exclude_logging: bool,
) -> FilterOptions {
    FilterOptions {
        exclude_filenames,
        exclude_functions,
        max_depth,
        exclude_logging,
    }
}

/// Filter a payload directly and extract its resulting frames list.
fn filter_and_extract_frames<'py>(
    py: Python<'py>,
    payload: &Bound<'py, PyDict>,
    options: &FilterOptions,
) -> PyResult<Bound<'py, PyList>> {
    let result = filter_payload(py, payload, options)?;
    let result_dict = result.cast_bound::<PyDict>(py)?;
    let frames = result_dict
        .get_item("frames")?
        .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err("frames key is None"))?;
    Ok(frames.cast::<PyList>()?.clone())
}

/// Extract `$dict[$key]` as `$ty`, panicking with a key-specific message at
/// each step of the lookup.
///
/// A macro rather than a helper function so panic line numbers point at the
/// calling test.
macro_rules! extract_dict_value {
    ($dict:expr, $key:expr, $ty:ty) => {{
        let key = $key;
        let value: $ty = $dict
            .get_item(key)
            .unwrap_or_else(|err| panic!("failed to get {key} key: {err}"))
            .unwrap_or_else(|| panic!("{key} key is None"))
            .extract()
            .unwrap_or_else(|err| panic!("failed to extract {key}: {err}"));
        value
    }};
}

/// Assert the `filename` of the frame at `index` within a frames list.
///
/// A macro rather than a helper function so panic line numbers point at the
/// calling test.
macro_rules! assert_frame_filename {
    ($frames_list:expr, $index:expr, $expected:expr) => {{
        let frame = $frames_list.get_item($index).expect("failed to get frame");
        let frame_dict = frame.cast::<PyDict>().expect("frame is not a dict");
        let filename = extract_dict_value!(frame_dict, "filename", String);
        assert_eq!(filename, $expected);
    }};
}

#[rstest]
#[serial]
fn filter_stack_payload_exclude_logging() {
    Python::attach(|py| {
        let payload = make_stack_payload_dict(
            py,
            &[
                "myapp/main.py",
                "femtologging/__init__.py",
                "logging/__init__.py",
            ],
        )
        .expect("payload should build");

        let frames_list =
            filter_and_extract_frames(py, &payload, &filter_options(None, None, None, true))
                .expect("filter should succeed");

        assert_eq!(frames_list.len(), 1);
        assert_frame_filename!(frames_list, 0, "myapp/main.py");
    });
}

#[rstest]
#[serial]
fn filter_stack_payload_exclude_filenames() {
    Python::attach(|py| {
        let payload = make_stack_payload_dict(
            py,
            &["myapp/main.py", ".venv/lib/requests.py", "myapp/utils.py"],
        )
        .expect("payload should build");

        let frames_list = filter_and_extract_frames(
            py,
            &payload,
            &filter_options(Some(vec![".venv/".to_owned()]), None, None, false),
        )
        .expect("filter should succeed");

        assert_eq!(frames_list.len(), 2);
    });
}

#[rstest]
#[serial]
fn filter_stack_payload_max_depth() {
    Python::attach(|py| {
        let payload = make_stack_payload_dict(py, &["a.py", "b.py", "c.py", "d.py", "e.py"])
            .expect("payload should build");

        let frames_list =
            filter_and_extract_frames(py, &payload, &filter_options(None, None, Some(2), false))
                .expect("filter should succeed");

        assert_eq!(frames_list.len(), 2);
        // Should be the last 2 frames (d.py, e.py)
        assert_frame_filename!(frames_list, 0, "d.py");
    });
}

#[rstest]
#[serial]
fn filter_exception_payload_detects_type() {
    Python::attach(|py| {
        let payload =
            make_exception_payload_dict(py, &["myapp/main.py", "femtologging/__init__.py"])
                .expect("payload should build");

        assert!(is_exception_payload(&payload).expect("is_exception_payload failed"));

        let result = filter_payload(py, &payload, &filter_options(None, None, None, true))
            .expect("filter_frames failed");
        let result_dict = result
            .cast_bound::<PyDict>(py)
            .expect("result is not a dict");

        // Should preserve exception fields
        let type_name = extract_dict_value!(result_dict, "type_name", String);
        assert_eq!(type_name, "ValueError");

        let frames = result_dict
            .get_item("frames")
            .expect("failed to get frames key")
            .expect("frames key is None");
        let frames_list = frames.cast::<PyList>().expect("frames is not a list");
        assert_eq!(frames_list.len(), 1);
    });
}

#[rstest]
#[serial]
fn filter_exception_payload_with_cause() {
    Python::attach(|py| {
        let cause = make_exception_payload_dict(py, &["cause.py", "femtologging/__init__.py"])
            .expect("payload should build");
        cause
            .set_item("type_name", "IOError")
            .expect("failed to set type_name");
        cause
            .set_item("message", "cause error")
            .expect("failed to set message");

        let payload = make_exception_payload_dict(py, &["main.py", "logging/__init__.py"])
            .expect("payload should build");
        payload
            .set_item("cause", cause)
            .expect("failed to set cause");

        let result = filter_payload(py, &payload, &filter_options(None, None, None, true))
            .expect("filter_frames failed");
        let result_dict = result
            .cast_bound::<PyDict>(py)
            .expect("result is not a dict");

        // Check main frames filtered
        let frames = result_dict
            .get_item("frames")
            .expect("failed to get frames key")
            .expect("frames key is None");
        let frames_list = frames.cast::<PyList>().expect("frames is not a list");
        assert_eq!(frames_list.len(), 1);

        // Check cause frames also filtered
        let cause_result = result_dict
            .get_item("cause")
            .expect("failed to get cause key")
            .expect("cause key is None");
        let cause_dict = cause_result.cast::<PyDict>().expect("cause is not a dict");
        let cause_frames = cause_dict
            .get_item("frames")
            .expect("failed to get cause frames key")
            .expect("cause frames key is None");
        let cause_frames_list = cause_frames
            .cast::<PyList>()
            .expect("cause frames is not a list");
        assert_eq!(cause_frames_list.len(), 1);
    });
}

#[rstest]
#[serial]
fn filter_stack_payload_exclude_functions() {
    Python::attach(|py| {
        let payload =
            make_stack_payload_dict(py, &["a.py", "b.py", "c.py"]).expect("payload should build");

        // Set function name on the second frame
        let frames = payload
            .get_item("frames")
            .expect("failed to get frames")
            .expect("frames is None");
        let frames_list = frames.cast::<PyList>().expect("frames is not a list");
        let frame1 = frames_list.get_item(1).expect("failed to get frame 1");
        let frame1_dict = frame1.cast::<PyDict>().expect("frame is not a dict");
        frame1_dict
            .set_item("function", "_internal_helper")
            .expect("failed to set function");

        let filtered_frames = filter_and_extract_frames(
            py,
            &payload,
            &filter_options(None, Some(vec!["_internal".to_owned()]), None, false),
        )
        .expect("filter should succeed");

        assert_eq!(filtered_frames.len(), 2);
    });
}

#[rstest]
#[serial]
fn filter_exception_payload_exclude_functions() {
    Python::attach(|py| {
        let payload = make_exception_payload_dict(py, &["a.py", "b.py", "c.py"])
            .expect("payload should build");

        // Set function name on the second frame
        let frames = payload
            .get_item("frames")
            .expect("failed to get frames")
            .expect("frames is None");
        let frames_list = frames.cast::<PyList>().expect("frames is not a list");
        let frame1 = frames_list.get_item(1).expect("failed to get frame 1");
        let frame1_dict = frame1.cast::<PyDict>().expect("frame is not a dict");
        frame1_dict
            .set_item("function", "_internal_helper")
            .expect("failed to set function");

        let result = filter_payload(
            py,
            &payload,
            &filter_options(None, Some(vec!["_internal".to_owned()]), None, false),
        )
        .expect("filter_frames failed");
        let result_dict = result
            .cast_bound::<PyDict>(py)
            .expect("result is not a dict");

        // Should preserve exception fields
        let type_name = extract_dict_value!(result_dict, "type_name", String);
        assert_eq!(type_name, "ValueError");

        let filtered_frames = result_dict
            .get_item("frames")
            .expect("failed to get frames key")
            .expect("frames key is None");
        let frames_result = filtered_frames
            .cast::<PyList>()
            .expect("frames is not a list");
        assert_eq!(frames_result.len(), 2);
    });
}

#[rstest]
#[case("frames_not_list", "must be a list")]
#[case("frame_not_dict", "must be a dict")]
#[serial]
fn filter_malformed_payload_raises_type_error(
    #[case] scenario: &str,
    #[case] expected_error_fragment: &str,
) {
    Python::attach(|py| {
        let payload = PyDict::new(py);
        payload
            .set_item("schema_version", 1u16)
            .expect("failed to set schema_version");

        // Set up the invalid payload based on scenario
        match scenario {
            "frames_not_list" => {
                payload
                    .set_item("frames", "not a list")
                    .expect("failed to set frames");
            }
            "frame_not_dict" => {
                let frames = PyList::empty(py);
                frames.append("not a dict").expect("failed to append");
                payload
                    .set_item("frames", frames)
                    .expect("failed to set frames");
            }
            _ => panic!("Unknown scenario"),
        }

        let result = filter_payload(py, &payload, &filter_options(None, None, None, false));
        let err = result.expect_err(&format!(
            "scenario '{scenario}' should fail with error containing '{expected_error_fragment}'"
        ));
        assert!(
            err.to_string().contains(expected_error_fragment),
            "error for scenario '{scenario}' should contain '{expected_error_fragment}', got: \
             {err}"
        );
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
    Python::attach(|py| {
        let payload = make_exception_payload_dict(py, &["main.py"]).expect("payload should build");
        payload
            .set_item("custom_field", "preserved_value")
            .expect("failed to set custom_field");
        payload
            .set_item("thread_id", 12345)
            .expect("failed to set thread_id");

        let result = filter_payload(py, &payload, &filter_options(None, None, None, false))
            .expect("filter_frames failed");
        let result_dict = result
            .cast_bound::<PyDict>(py)
            .expect("result is not a dict");

        // Check custom fields are preserved
        let custom_field = extract_dict_value!(result_dict, "custom_field", String);
        assert_eq!(custom_field, "preserved_value");

        let thread_id = extract_dict_value!(result_dict, "thread_id", i32);
        assert_eq!(thread_id, 12345);
    });
}
