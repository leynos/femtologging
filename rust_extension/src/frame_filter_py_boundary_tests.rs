//! Python call-boundary regression tests for frame filtering.

use super::*;
use pyo3::types::{PyDict, PyList, PyModule};
use rstest::rstest;
use serial_test::serial;

fn make_stack_payload_dict<'py>(
    py: Python<'py>,
    filenames: &[&str],
) -> PyResult<Bound<'py, PyDict>> {
    let frames = PyList::empty(py);
    for (index, filename) in filenames.iter().enumerate() {
        let frame = PyDict::new(py);
        frame.set_item("filename", *filename)?;
        let line_number = u32::try_from(index + 1)
            .map_err(|_| pyo3::exceptions::PyOverflowError::new_err("frame index exceeds u32"))?;
        frame.set_item("lineno", line_number)?;
        frame.set_item("function", format!("function_{index}"))?;
        frames.append(frame)?;
    }
    let payload = PyDict::new(py);
    payload.set_item("schema_version", 1u16)?;
    payload.set_item("frames", frames)?;
    Ok(payload)
}

fn python_filter_function<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
    let module = PyModule::new(py, "frame_filter_boundary")?;
    module.add_function(wrap_pyfunction!(filter_frames, &module)?)?;
    Ok(module.getattr("filter_frames")?)
}

#[rstest]
#[serial]
fn filter_frames_python_boundary_preserves_signature_and_defaults() {
    Python::attach(|py| {
        let function = python_filter_function(py).expect("Python function should build");
        let signature: String = function
            .getattr("__text_signature__")
            .expect("text signature should be available")
            .extract()
            .expect("text signature should be text");
        assert_eq!(
            signature,
            "(payload, *, exclude_filenames=None, exclude_functions=None, max_depth=None, exclude_logging=False)"
        );

        let payload = make_stack_payload_dict(py, &["main.py", "logging/__init__.py"])
            .expect("payload should build");
        let default_result = function
            .call1((&payload,))
            .expect("default Python call should succeed")
            .cast_into::<PyDict>()
            .expect("default result should be a dict");
        let default_frames = default_result
            .get_item("frames")
            .expect("default result lookup should succeed")
            .expect("default result should contain frames")
            .cast_into::<PyList>()
            .expect("default frames should be a list");
        assert_eq!(default_frames.len(), 2);

        let keywords = PyDict::new(py);
        keywords
            .set_item("payload", &payload)
            .expect("payload keyword should be set");
        keywords
            .set_item("exclude_logging", true)
            .expect("filter keyword should be set");
        let keyword_result = function
            .call((), Some(&keywords))
            .expect("keyword Python call should succeed")
            .cast_into::<PyDict>()
            .expect("keyword result should be a dict");
        let keyword_frames = keyword_result
            .get_item("frames")
            .expect("keyword result lookup should succeed")
            .expect("keyword result should contain frames")
            .cast_into::<PyList>()
            .expect("keyword frames should be a list");
        assert_eq!(keyword_frames.len(), 1);
    });
}

#[rstest]
#[serial]
fn filter_frames_python_boundary_rejects_invalid_argument_shapes() {
    Python::attach(|py| {
        let function = python_filter_function(py).expect("Python function should build");
        let payload = make_stack_payload_dict(py, &["main.py"]).expect("payload should build");

        let missing_error = function.call0().expect_err("payload should be required");
        assert!(
            missing_error
                .to_string()
                .contains("missing 1 required positional argument: 'payload'"),
            "unexpected missing-argument error: {missing_error}"
        );

        let positional_error = function
            .call1((&payload, false))
            .expect_err("optional arguments must be keyword-only");
        assert!(
            positional_error
                .to_string()
                .contains("takes 1 positional argument but 2 were given"),
            "unexpected positional-argument error: {positional_error}"
        );

        let duplicate_keywords = PyDict::new(py);
        duplicate_keywords
            .set_item("payload", &payload)
            .expect("duplicate payload keyword should be set");
        let duplicate_error = function
            .call((&payload,), Some(&duplicate_keywords))
            .expect_err("payload must not be supplied twice");
        assert!(
            duplicate_error
                .to_string()
                .contains("got multiple values for argument 'payload'"),
            "unexpected duplicate-argument error: {duplicate_error}"
        );

        let unknown_keywords = PyDict::new(py);
        unknown_keywords
            .set_item("unrecognized", true)
            .expect("unknown keyword should be set");
        let unknown_error = function
            .call((&payload,), Some(&unknown_keywords))
            .expect_err("unknown keyword must be rejected");
        assert!(
            unknown_error
                .to_string()
                .contains("got an unexpected keyword argument 'unrecognized'"),
            "unexpected unknown-keyword error: {unknown_error}"
        );
    });
}
