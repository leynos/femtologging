//! Regression tests for iterative exception-payload graph traversal.

use super::*;
use pyo3::exceptions::PyValueError;

/// Frame and exception-graph bounds used by the exception traversal tests.
#[derive(Clone, Copy)]
struct GraphFilterLimits {
    /// Maximum frames retained for each payload.
    frame_max_depth: Option<usize>,
    /// Maximum edge depth permitted below the root payload.
    exception_max_depth: usize,
    /// Maximum number of payload occurrences allowed, including the root.
    exception_max_nodes: usize,
}

impl GraphFilterLimits {
    /// Create a complete set of frame and graph bounds for one test.
    const fn new(
        frame_max_depth: Option<usize>,
        exception_max_depth: usize,
        exception_max_nodes: usize,
    ) -> Self {
        Self {
            frame_max_depth,
            exception_max_depth,
            exception_max_nodes,
        }
    }
}

fn filter_with_limits<'py>(
    py: Python<'py>,
    payload: &Bound<'py, PyDict>,
    limits: GraphFilterLimits,
) -> PyResult<Bound<'py, PyDict>> {
    let filtered = filter_frames(
        py,
        payload,
        None,
        None,
        limits.frame_max_depth,
        false,
        limits.exception_max_depth,
        limits.exception_max_nodes,
    )?;
    let filtered_dict = filtered.cast_bound::<PyDict>(py)?;
    Ok(filtered_dict.clone())
}

fn assert_value_error(py: Python<'_>, error: &PyErr, message: &str) {
    assert!(
        error.is_instance_of::<PyValueError>(py),
        "expected ValueError, got {error}"
    );
    assert!(
        error.to_string().contains(message),
        "expected {message:?} in {error}"
    );
}

#[rstest]
#[case::cause("cause", "$.cause")]
#[case::context("context", "$.context")]
#[case::exception_group("exceptions", "$.exceptions[0]")]
#[serial]
fn filter_exception_graph_rejects_self_cycles(#[case] edge: &str, #[case] path: &str) {
    Python::attach(|py| {
        let payload = make_exception_payload_dict(py, &[]).expect("payload should build");
        match edge {
            "exceptions" => {
                let exceptions = PyList::empty(py);
                exceptions
                    .append(payload.clone())
                    .expect("self-reference should append");
                payload
                    .set_item(edge, exceptions)
                    .expect("exception group should attach");
            }
            _ => payload
                .set_item(edge, payload.clone())
                .expect("self-reference should attach"),
        }

        let error = filter_with_limits(py, &payload, GraphFilterLimits::new(None, 1, 10))
            .expect_err("a self-cycle must be rejected");
        assert_value_error(
            py,
            &error,
            &format!("exception payload graph contains a cycle at {path}"),
        );
    });
}

#[rstest]
#[serial]
fn filter_exception_graph_rejects_two_node_cycle() {
    Python::attach(|py| {
        let first = make_exception_payload_dict(py, &[]).expect("first payload should build");
        let second = make_exception_payload_dict(py, &[]).expect("second payload should build");
        first
            .set_item("cause", second.clone())
            .expect("cause should attach");
        second
            .set_item("context", first.clone())
            .expect("context should attach");

        let error = filter_with_limits(py, &first, GraphFilterLimits::new(None, 2, 10))
            .expect_err("a two-node cycle must be rejected");
        assert_value_error(
            py,
            &error,
            "exception payload graph contains a cycle at $.cause.context",
        );
    });
}

#[rstest]
#[serial]
fn filter_exception_graph_reports_cycle_below_valid_prefix() {
    Python::attach(|py| {
        let root = make_exception_payload_dict(py, &[]).expect("root should build");
        let middle = make_exception_payload_dict(py, &[]).expect("middle should build");
        let leaf = make_exception_payload_dict(py, &[]).expect("leaf should build");
        let exceptions = PyList::empty(py);
        exceptions.append(leaf.clone()).expect("leaf should append");
        root.set_item("cause", middle.clone())
            .expect("cause should attach");
        middle
            .set_item("exceptions", exceptions)
            .expect("group should attach");
        leaf.set_item("context", root.clone())
            .expect("context should attach");

        let error = filter_with_limits(py, &root, GraphFilterLimits::new(None, 3, 10))
            .expect_err("a cycle below a valid prefix must be rejected");
        assert_value_error(
            py,
            &error,
            "exception payload graph contains a cycle at $.cause.exceptions[0].context",
        );
    });
}

#[rstest]
#[serial]
fn filter_exception_graph_copies_shared_cause_and_context_per_occurrence() {
    Python::attach(|py| {
        let root = make_exception_payload_dict(py, &[]).expect("root should build");
        let shared =
            make_exception_payload_dict(py, &["shared.py"]).expect("shared payload should build");
        root.set_item("cause", shared.clone())
            .expect("cause should attach");
        root.set_item("context", shared)
            .expect("context should attach");

        let filtered = filter_with_limits(py, &root, GraphFilterLimits::new(None, 1, 3))
            .expect("shared acyclic payloads should be accepted");
        let cause_value = filtered
            .get_item("cause")
            .expect("cause lookup should succeed")
            .expect("cause should be present");
        let cause = cause_value
            .cast::<PyDict>()
            .expect("cause should be a dictionary")
            .clone();
        let context_value = filtered
            .get_item("context")
            .expect("context lookup should succeed")
            .expect("context should be present");
        let context = context_value
            .cast::<PyDict>()
            .expect("context should be a dictionary")
            .clone();

        assert_ne!(cause.as_ptr(), context.as_ptr());
        assert_eq!(
            cause
                .get_item("frames")
                .expect("frame lookup should succeed")
                .expect("frames should be present")
                .cast::<PyList>()
                .expect("frames should be a list")
                .len(),
            1
        );
    });
}

#[rstest]
#[serial]
fn filter_exception_graph_copies_shared_group_member_per_occurrence() {
    Python::attach(|py| {
        let root = make_exception_payload_dict(py, &[]).expect("root should build");
        let shared =
            make_exception_payload_dict(py, &["shared.py"]).expect("shared payload should build");
        let exceptions = PyList::empty(py);
        exceptions
            .append(shared.clone())
            .expect("first member should append");
        exceptions
            .append(shared)
            .expect("second member should append");
        root.set_item("exceptions", exceptions)
            .expect("group should attach");

        let filtered = filter_with_limits(py, &root, GraphFilterLimits::new(None, 1, 3))
            .expect("shared group members should be accepted");
        let exceptions_value = filtered
            .get_item("exceptions")
            .expect("group lookup should succeed")
            .expect("group should be present");
        let exceptions = exceptions_value
            .cast::<PyList>()
            .expect("group should be a list")
            .clone();
        let first_value = exceptions.get_item(0).expect("first member should exist");
        let first = first_value
            .cast::<PyDict>()
            .expect("first member should be a dictionary")
            .clone();
        let second_value = exceptions.get_item(1).expect("second member should exist");
        let second = second_value
            .cast::<PyDict>()
            .expect("second member should be a dictionary")
            .clone();

        assert_ne!(first.as_ptr(), second.as_ptr());
    });
}

#[rstest]
#[serial]
fn filter_exception_graph_enforces_depth_and_node_boundaries() {
    Python::attach(|py| {
        let root = make_exception_payload_dict(py, &["root.py"]).expect("root should build");
        let child = make_exception_payload_dict(py, &["child.py"]).expect("child should build");
        root.set_item("cause", child).expect("cause should attach");

        let exact_depth = filter_with_limits(py, &root, GraphFilterLimits::new(Some(1), 1, 2))
            .expect("depth-one graph should pass");
        assert_eq!(
            exact_depth
                .get_item("frames")
                .expect("frames lookup should succeed")
                .expect("root frames should remain")
                .cast::<PyList>()
                .expect("root frames should be a list")
                .len(),
            1
        );

        let depth_error = filter_with_limits(py, &root, GraphFilterLimits::new(None, 0, 2))
            .expect_err("a cause exceeds a zero-depth budget");
        assert_value_error(py, &depth_error, "max_exception_depth=0");

        let root_only = make_exception_payload_dict(py, &[]).expect("root should build");
        filter_with_limits(py, &root_only, GraphFilterLimits::new(None, 0, 1))
            .expect("a root at depth zero should pass");

        let node_error = filter_with_limits(py, &root, GraphFilterLimits::new(None, 1, 1))
            .expect_err("the cause exceeds a one-node budget");
        assert_value_error(py, &node_error, "max_exception_nodes=1");

        let zero_node_error =
            filter_with_limits(py, &root_only, GraphFilterLimits::new(None, 0, 0))
                .expect_err("zero nodes are not a valid exception budget");
        assert_value_error(py, &zero_node_error, "max_exception_nodes=0");
    });
}

#[rstest]
#[serial]
fn filter_exception_graph_depth_limit_is_independent_of_frame_depth() {
    Python::attach(|py| {
        let root =
            make_exception_payload_dict(py, &["a.py", "b.py", "c.py"]).expect("root should build");
        root.set_item(
            "cause",
            make_exception_payload_dict(py, &["d.py", "e.py"]).expect("cause should build"),
        )
        .expect("cause should attach");

        let filtered = filter_with_limits(py, &root, GraphFilterLimits::new(Some(1), 1, 2))
            .expect("both payloads should fit their independent limits");
        let frames_value = filtered
            .get_item("frames")
            .expect("root frame lookup should succeed")
            .expect("root frames should be present");
        let frames = frames_value
            .cast::<PyList>()
            .expect("root frames should be a list")
            .clone();
        let cause_value = filtered
            .get_item("cause")
            .expect("cause lookup should succeed")
            .expect("cause should be present");
        let cause = cause_value
            .cast::<PyDict>()
            .expect("cause should be a dictionary")
            .clone();
        let cause_frames_value = cause
            .get_item("frames")
            .expect("cause frames lookup should succeed")
            .expect("cause frames should be present");
        let cause_frames = cause_frames_value
            .cast::<PyList>()
            .expect("cause frames should be a list")
            .clone();

        assert_eq!(frames.len(), 1);
        assert_eq!(cause_frames.len(), 1);
    });
}
