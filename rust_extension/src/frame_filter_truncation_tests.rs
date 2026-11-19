//! Tests that frame filtering preserves capture truncation markers.
//!
//! Filtering changes which frames a consumer sees; it cannot undo an omission
//! made during capture. These tests pin the pass-through so a future rewrite of
//! the payload cloning cannot quietly erase the marker.

use super::test_helpers::{TestFilterOptions, filter_frames_with_defaults};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use serial_test::serial;

/// Build a frame dict.
fn frame<'py>(py: Python<'py>, filename: &str) -> PyResult<Bound<'py, PyDict>> {
    let frame = PyDict::new(py);
    frame.set_item("filename", filename)?;
    frame.set_item("lineno", 1u32)?;
    frame.set_item("function", "work")?;
    Ok(frame)
}

/// Build a truncation marker dict.
fn truncation<'py>(
    py: Python<'py>,
    frames_omitted: u32,
    cause_omitted: bool,
) -> PyResult<Bound<'py, PyDict>> {
    let marker = PyDict::new(py);
    marker.set_item("frames_omitted", frames_omitted)?;
    if cause_omitted {
        marker.set_item("cause_omitted", true)?;
    }
    Ok(marker)
}

/// Return the `frames_omitted` count carried by `payload["truncation"]`.
fn frames_omitted(payload: &Bound<'_, PyDict>) -> PyResult<u32> {
    payload
        .get_item("truncation")?
        .expect("truncation key should survive filtering")
        .cast::<PyDict>()?
        .get_item("frames_omitted")?
        .expect("frames_omitted should survive filtering")
        .extract()
}

#[test]
#[serial]
fn stack_payload_preserves_truncation_marker() -> PyResult<()> {
    Python::attach(|py| {
        let frames = PyList::empty(py);
        frames.append(frame(py, "app.py")?)?;
        frames.append(frame(py, "lib.py")?)?;
        let payload = PyDict::new(py);
        payload.set_item("schema_version", 1u16)?;
        payload.set_item("frames", frames)?;
        payload.set_item("truncation", truncation(py, 7, false)?)?;

        let result = filter_frames_with_defaults(
            py,
            &payload,
            TestFilterOptions {
                max_depth: Some(1),
                ..TestFilterOptions::default()
            },
        )?;
        let result = result.cast_bound::<PyDict>(py)?;

        assert_eq!(frames_omitted(result)?, 7);
        assert_eq!(
            result
                .get_item("frames")?
                .expect("one frame should be retained")
                .cast::<PyList>()?
                .len(),
            1
        );
        Ok(())
    })
}

/// Build a two-frame list for a nested payload.
fn two_frames<'py>(py: Python<'py>, prefix: &str) -> PyResult<Bound<'py, PyList>> {
    let frames = PyList::empty(py);
    frames.append(frame(py, &format!("{prefix}_app.py"))?)?;
    frames.append(frame(py, &format!("{prefix}_lib.py"))?)?;
    Ok(frames)
}

/// Return the number of frames retained in `payload`.
fn frame_count(payload: &Bound<'_, PyDict>) -> PyResult<usize> {
    Ok(payload
        .get_item("frames")?
        .expect("filtered payload should still carry frames")
        .cast::<PyList>()?
        .len())
}

#[test]
#[serial]
fn exception_payload_preserves_markers_at_every_level() -> PyResult<()> {
    Python::attach(|py| {
        let frames = PyList::empty(py);
        frames.append(frame(py, "app.py")?)?;

        // Every nested payload starts with two frames, so the depth ceiling has
        // something to remove. Otherwise the test would pass even if the filter
        // left nested payloads untouched.
        let cause = PyDict::new(py);
        cause.set_item("schema_version", 1u16)?;
        cause.set_item("type_name", "OSError")?;
        cause.set_item("message", "root")?;
        cause.set_item("truncation", truncation(py, 2, false)?)?;
        cause.set_item("frames", two_frames(py, "cause")?)?;

        let context = PyDict::new(py);
        context.set_item("schema_version", 1u16)?;
        context.set_item("type_name", "RuntimeError")?;
        context.set_item("message", "handled")?;
        context.set_item("truncation", truncation(py, 4, false)?)?;
        context.set_item("frames", two_frames(py, "context")?)?;

        let member = PyDict::new(py);
        member.set_item("schema_version", 1u16)?;
        member.set_item("type_name", "ValueError")?;
        member.set_item("message", "member")?;
        member.set_item("truncation", truncation(py, 1, false)?)?;
        member.set_item("frames", two_frames(py, "member")?)?;

        let payload = PyDict::new(py);
        payload.set_item("schema_version", 1u16)?;
        payload.set_item("type_name", "ExceptionGroup")?;
        payload.set_item("message", "several")?;
        payload.set_item("frames", frames)?;
        payload.set_item("truncation", truncation(py, 3, true)?)?;
        payload.set_item("cause", cause)?;
        payload.set_item("context", context)?;
        payload.set_item("exceptions", {
            let members = PyList::empty(py);
            members.append(member)?;
            members
        })?;

        let result = filter_frames_with_defaults(
            py,
            &payload,
            TestFilterOptions {
                max_depth: Some(1),
                ..TestFilterOptions::default()
            },
        )?;
        let result = result.cast_bound::<PyDict>(py)?;

        assert_eq!(frames_omitted(result)?, 3);
        let cause_omitted: bool = result
            .get_item("truncation")?
            .expect("root marker should survive")
            .cast::<PyDict>()?
            .get_item("cause_omitted")?
            .expect("cause_omitted should survive")
            .extract()?;
        assert!(cause_omitted, "cause_omitted flag should survive");

        let cause = result
            .get_item("cause")?
            .expect("cause should survive")
            .cast::<PyDict>()?
            .clone();
        assert_eq!(frames_omitted(&cause)?, 2);
        assert_eq!(frame_count(&cause)?, 1, "the cause should be filtered");

        let context = result
            .get_item("context")?
            .expect("context should survive")
            .cast::<PyDict>()?
            .clone();
        assert_eq!(frames_omitted(&context)?, 4);
        assert_eq!(frame_count(&context)?, 1, "the context should be filtered");

        let member = result
            .get_item("exceptions")?
            .expect("group members should survive")
            .cast::<PyList>()?
            .get_item(0)?
            .cast::<PyDict>()?
            .clone();
        assert_eq!(frames_omitted(&member)?, 1);
        assert_eq!(frame_count(&member)?, 1, "the member should be filtered");
        Ok(())
    })
}

#[test]
#[serial]
fn marker_survives_removal_of_all_frames() -> PyResult<()> {
    Python::attach(|py| {
        let frames = PyList::empty(py);
        frames.append(frame(py, "app.py")?)?;
        let payload = PyDict::new(py);
        payload.set_item("schema_version", 1u16)?;
        payload.set_item("frames", frames)?;
        payload.set_item("truncation", truncation(py, 9, false)?)?;

        let result = filter_frames_with_defaults(
            py,
            &payload,
            TestFilterOptions {
                max_depth: Some(0),
                ..TestFilterOptions::default()
            },
        )?;
        let result = result.cast_bound::<PyDict>(py)?;

        assert!(
            result.get_item("frames")?.is_none(),
            "an emptied frames key is removed, matching serde"
        );
        assert_eq!(frames_omitted(result)?, 9);
        Ok(())
    })
}
