//! Boundary tests for the recursive validation traversal limits.
//!
//! Validation walks a payload tree depth-first, bounded by
//! [`MAX_EXCEPTION_PAYLOAD_DEPTH`] and [`MAX_EXCEPTION_PAYLOAD_NODES`]. These
//! tests pin both sides of each boundary across every edge kind, so an
//! off-by-one in either check fails, and pin the validation limits to the
//! frame filter's defaults. The path, ordering and version-mismatch tests live
//! in `nested_version_tests`.

use crate::exception_schema::{
    ExceptionPayload, MAX_EXCEPTION_PAYLOAD_DEPTH, MAX_EXCEPTION_PAYLOAD_NODES, SchemaVersionError,
    SchemaVersioned,
};
use rstest::rstest;

use super::deep_chain_tests::{ChainKind, build_exception_chain};

/// A chain one node longer than the depth limit still validates.
#[rstest]
fn a_chain_at_the_depth_limit_validates() {
    let payload = build_exception_chain(ChainKind::Cause, MAX_EXCEPTION_PAYLOAD_DEPTH + 1);

    payload
        .validate_version()
        .expect("the deepest payload sits exactly at the depth limit");
}

/// Assert that validating reported `TreeTooDeep` along one repeated edge.
#[track_caller]
fn assert_depth_limit(result: Result<(), SchemaVersionError>, expected_segment: &str) {
    let error = result.expect_err("the chain is deeper than the limit");

    match error {
        SchemaVersionError::TreeTooDeep { path, max_depth } => {
            assert_eq!(max_depth, MAX_EXCEPTION_PAYLOAD_DEPTH);
            let segments: Vec<&str> = path.split('.').collect();
            assert_eq!(
                segments.len(),
                MAX_EXCEPTION_PAYLOAD_DEPTH + 1,
                "the path should name the first payload past the limit"
            );
            assert!(
                segments.iter().all(|segment| *segment == expected_segment),
                "unexpected path: {path}"
            );
        }
        other => panic!("expected TreeTooDeep, got {other:?}"),
    }
}

/// A cause chain past the depth limit reports the limit and the path.
#[rstest]
fn a_cause_chain_past_the_depth_limit_reports_the_limit() {
    let payload = build_exception_chain(ChainKind::Cause, MAX_EXCEPTION_PAYLOAD_DEPTH + 2);

    assert_depth_limit(payload.validate_version(), "cause");
}

/// A context chain is bounded by the same depth limit.
#[rstest]
fn a_context_chain_past_the_depth_limit_reports_the_limit() {
    let payload = build_exception_chain(ChainKind::Context, MAX_EXCEPTION_PAYLOAD_DEPTH + 2);

    assert_depth_limit(payload.validate_version(), "context");
}

/// Wrap `leaf` in `depth` nested exception groups, each holding one member.
fn nested_group_chain(leaf: ExceptionPayload, depth: usize) -> ExceptionPayload {
    (0..depth).fold(leaf, |inner, _| ExceptionPayload {
        exceptions: vec![inner],
        ..ExceptionPayload::new("Group", "group")
    })
}

/// Nested exception-group edges are bounded by the same depth limit.
#[rstest]
fn a_group_chain_past_the_depth_limit_reports_the_limit() {
    let leaf = ExceptionPayload::new("Leaf", "leaf");
    let payload = nested_group_chain(leaf, MAX_EXCEPTION_PAYLOAD_DEPTH + 2);

    assert_depth_limit(payload.validate_version(), "exceptions[0]");
}

/// Build a group payload with `total` nodes: the group plus its members.
fn group_with_nodes(total: usize) -> ExceptionPayload {
    assert!(total >= 1, "a payload tree holds at least the root");

    ExceptionPayload {
        exceptions: (1..total)
            .map(|index| ExceptionPayload::new(format!("Member{index}"), "member"))
            .collect(),
        ..ExceptionPayload::new("Group", "group")
    }
}

/// A tree holding exactly the node limit validates.
#[rstest]
fn a_tree_at_the_node_limit_validates() {
    group_with_nodes(MAX_EXCEPTION_PAYLOAD_NODES)
        .validate_version()
        .expect("the tree holds exactly the node limit");
}

/// Assert that validating reported `TreeTooLarge` with the node limit.
#[track_caller]
fn assert_node_limit(result: Result<(), SchemaVersionError>) {
    let error = result.expect_err("the tree exceeds the node limit");

    match error {
        SchemaVersionError::TreeTooLarge { max_nodes } => {
            assert_eq!(max_nodes, MAX_EXCEPTION_PAYLOAD_NODES);
        }
        other => panic!("expected TreeTooLarge, got {other:?}"),
    }
}

/// A tree holding one node more than the limit reports the limit.
#[rstest]
fn a_tree_past_the_node_limit_reports_the_limit() {
    assert_node_limit(group_with_nodes(MAX_EXCEPTION_PAYLOAD_NODES + 1).validate_version());
}

/// The node budget counts payloads below a cause or context edge too.
#[rstest]
#[case(true)]
#[case(false)]
fn a_node_limit_below_a_chain_edge_is_reported(#[case] below_cause: bool) {
    let group = group_with_nodes(MAX_EXCEPTION_PAYLOAD_NODES);
    let root = ExceptionPayload::new("Root", "root");
    let payload = if below_cause {
        root.with_cause(group)
    } else {
        root.with_context(group)
    };

    assert_node_limit(payload.validate_version());
}

/// Validation and recursive frame filtering must start from the same bounds.
///
/// A payload the frame filter processes should not be rejected by schema
/// validation for exceeding a traversal limit the filter starts from. The
/// limits are budgets, not equivalent gates: validation also rejects an
/// unsupported `schema_version`, which filtering never inspects.
#[cfg(feature = "python")]
#[rstest]
fn limits_match_recursive_frame_processing() {
    use crate::frame_filter_py::{DEFAULT_MAX_EXCEPTION_DEPTH, DEFAULT_MAX_EXCEPTION_NODES};

    assert_eq!(
        MAX_EXCEPTION_PAYLOAD_DEPTH, DEFAULT_MAX_EXCEPTION_DEPTH,
        "validation depth and frame-filter depth must stay aligned"
    );
    assert_eq!(
        MAX_EXCEPTION_PAYLOAD_NODES, DEFAULT_MAX_EXCEPTION_NODES,
        "validation and frame-filter node limits must stay aligned"
    );
}
