//! Tests for recursive schema version validation.
//!
//! These tests cover every nested location that validation reaches, the
//! depth-first pre-order in which failures are reported, both traversal
//! bounds, and the path-bearing errors that locate a nested mismatch. The
//! root-level tests in `schema_tests` are left unchanged so they keep pinning
//! the behaviour that predates recursive validation.

use crate::exception_schema::{
    EXCEPTION_SCHEMA_VERSION, ExceptionPayload, MAX_EXCEPTION_PAYLOAD_DEPTH,
    MAX_EXCEPTION_PAYLOAD_NODES, MIN_EXCEPTION_SCHEMA_VERSION, SchemaVersionError, SchemaVersioned,
};
use rstest::rstest;

use super::deep_chain_tests::{ChainKind, build_exception_chain};

/// A schema version one newer than this build supports.
const TOO_NEW_VERSION: u16 = EXCEPTION_SCHEMA_VERSION + 1;

/// The kind of edge that leads from a payload to a nested payload.
///
/// Modelling the edge as an enum keeps an invalid link kind unrepresentable,
/// so the wrapping helper needs no fallible catch-all arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LinkKind {
    /// A `cause` edge.
    Cause,
    /// A `context` edge.
    Context,
    /// The exception-group member at the given index.
    Exceptions(usize),
}

impl LinkKind {
    /// Return the path segment this link contributes.
    fn segment(self) -> String {
        match self {
            Self::Cause => "cause".to_owned(),
            Self::Context => "context".to_owned(),
            Self::Exceptions(index) => format!("exceptions[{index}]"),
        }
    }

    /// Wrap `leaf` in `depth` links of this kind.
    ///
    /// The links are added outside `leaf`, so `leaf` sits at depth `depth`.
    fn wrap(self, leaf: ExceptionPayload, depth: usize) -> ExceptionPayload {
        (0..depth).fold(leaf, |inner, _| match self {
            Self::Cause => ExceptionPayload::new("CauseWrapper", "cause").with_cause(inner),
            Self::Context => ExceptionPayload::new("ContextWrapper", "context").with_context(inner),
            Self::Exceptions(index) => ExceptionPayload {
                exceptions: group_members(index, inner),
                ..ExceptionPayload::new("GroupWrapper", "group")
            },
        })
    }
}

/// Place `member` at `index` among supported siblings.
fn group_members(index: usize, member: ExceptionPayload) -> Vec<ExceptionPayload> {
    (0..=index)
        .map(|position| {
            if position == index {
                member.clone()
            } else {
                ExceptionPayload::new("Sibling", "sibling")
            }
        })
        .collect()
}

/// Return `payload` carrying `version` as its schema version.
fn with_version(mut payload: ExceptionPayload, version: u16) -> ExceptionPayload {
    payload.schema_version = version;
    payload
}

/// Return the mismatch `validate_schema_version` reports for `version`.
fn expected_mismatch(version: u16) -> SchemaVersionError {
    if version > EXCEPTION_SCHEMA_VERSION {
        SchemaVersionError::VersionTooNew {
            found: version,
            max_supported: EXCEPTION_SCHEMA_VERSION,
        }
    } else {
        SchemaVersionError::VersionTooOld {
            found: version,
            min_supported: MIN_EXCEPTION_SCHEMA_VERSION,
        }
    }
}

/// Return the version carried by a plain mismatch error.
fn found_version(error: &SchemaVersionError) -> u16 {
    match error {
        SchemaVersionError::VersionTooNew { found, .. }
        | SchemaVersionError::VersionTooOld { found, .. } => *found,
        other => panic!("expected a version mismatch, got {other:?}"),
    }
}

/// Assert that validating reported `expected` at `path`.
#[track_caller]
fn assert_nested_failure(
    result: Result<(), SchemaVersionError>,
    path: &str,
    expected: SchemaVersionError,
) {
    let error = result.expect_err("an unsupported nested version must fail");
    match &error {
        SchemaVersionError::Nested {
            path: actual_path,
            source,
        } => {
            assert_eq!(actual_path, path, "unexpected failing path");
            assert_eq!(**source, expected, "unexpected nested version error");
        }
        other => panic!("expected Nested at {path}, got {other:?}"),
    }

    let message = error.to_string();
    assert!(
        message.contains(path),
        "message should name the failing path: {message}"
    );
    let found = found_version(&expected);
    assert!(
        message.contains(&format!("found {found}")),
        "message should name the found version: {message}"
    );
}

/// Every nested location and depth reports the path to the failing payload.
#[rstest]
fn every_nested_location_reports_its_path(
    #[values(
        LinkKind::Cause,
        LinkKind::Context,
        LinkKind::Exceptions(0),
        LinkKind::Exceptions(3)
    )]
    link: LinkKind,
    #[values(0, TOO_NEW_VERSION, u16::MAX)] version: u16,
    #[values(1, 2, 5)] depth: usize,
) {
    let leaf = with_version(ExceptionPayload::new("Leaf", "leaf"), version);
    let payload = link.wrap(leaf, depth);

    let expected_path = vec![link.segment(); depth].join(".");
    assert_nested_failure(
        payload.validate_version(),
        &expected_path,
        expected_mismatch(version),
    );
}

/// A path that crosses every edge kind names each segment in order.
#[rstest]
#[case(TOO_NEW_VERSION)]
#[case(0)]
fn a_mixed_path_names_every_edge(#[case] version: u16) {
    let bad = with_version(ExceptionPayload::new("Bad", "bad"), version);
    let member = ExceptionPayload::new("Member", "member").with_context(bad);
    let group = ExceptionPayload {
        exceptions: group_members(2, member),
        ..ExceptionPayload::new("Group", "group")
    };
    let root = ExceptionPayload::new("Root", "root").with_cause(group);

    assert_nested_failure(
        root.validate_version(),
        "cause.exceptions[2].context",
        expected_mismatch(version),
    );
}

/// A group member can itself hold a cause and a context.
#[rstest]
fn a_group_member_descendant_path_is_reported() {
    let bad = with_version(ExceptionPayload::new("Bad", "bad"), 0);
    let cause = ExceptionPayload::new("Cause", "cause").with_context(bad);
    let root = ExceptionPayload {
        exceptions: vec![ExceptionPayload::new("Member", "member").with_cause(cause)],
        ..ExceptionPayload::new("Group", "group")
    };

    assert_nested_failure(
        root.validate_version(),
        "exceptions[0].cause.context",
        expected_mismatch(0),
    );
}

/// A tree whose every payload is supported validates successfully.
#[rstest]
fn a_fully_supported_tree_validates() {
    let deep_cause = ExceptionPayload::new("DeepCause", "deep")
        .with_cause(ExceptionPayload::new("DeepestCause", "deepest"));
    let cause = ExceptionPayload::new("Cause", "cause")
        .with_cause(deep_cause)
        .with_context(ExceptionPayload::new("CauseContext", "cause context"));
    let context = ExceptionPayload::new("Context", "context")
        .with_context(ExceptionPayload::new("DeepContext", "deep context"));
    let members = vec![
        ExceptionPayload::new("Member0", "m0")
            .with_cause(ExceptionPayload::new("Member0Cause", "cause")),
        ExceptionPayload::new("Member1", "m1")
            .with_context(ExceptionPayload::new("Member1Context", "context")),
        ExceptionPayload::new("Member2", "m2").with_cause(
            ExceptionPayload::new("Member2Cause", "cause")
                .with_context(ExceptionPayload::new("Member2Context", "context")),
        ),
    ];
    let root = ExceptionPayload {
        exceptions: members,
        ..ExceptionPayload::new("Root", "root")
            .with_cause(cause)
            .with_context(context)
    };

    root.validate_version()
        .expect("every nested schema version is supported");
}

/// A deserialized payload reports the path to its unsupported member.
#[rstest]
fn a_deserialized_tree_reports_the_nested_future_version() {
    let json = r#"{
        "schema_version": 1,
        "type_name": "ExceptionGroup",
        "message": "group",
        "exceptions": [
            {"schema_version": 1, "type_name": "Handled", "message": "handled"},
            {"schema_version": 999, "type_name": "Future", "message": "from the future"}
        ]
    }"#;

    let payload: ExceptionPayload =
        serde_json::from_str(json).expect("deserialization should succeed");

    assert_nested_failure(
        payload.validate_version(),
        "exceptions[1]",
        SchemaVersionError::VersionTooNew {
            found: 999,
            max_supported: EXCEPTION_SCHEMA_VERSION,
        },
    );
}

/// An unsupported root is reported plainly rather than wrapped.
#[rstest]
fn the_root_is_reported_before_its_descendants() {
    let payload = with_version(ExceptionPayload::new("Root", "root"), 0).with_cause(with_version(
        ExceptionPayload::new("Cause", "cause"),
        TOO_NEW_VERSION,
    ));

    let error = payload
        .validate_version()
        .expect_err("the root version is unsupported");

    assert_eq!(
        error,
        expected_mismatch(0),
        "a root mismatch should not be wrapped in Nested"
    );
}

/// The cause edge is visited before the context edge.
#[rstest]
fn cause_is_reported_before_context() {
    let payload = ExceptionPayload::new("Root", "root")
        .with_cause(with_version(
            ExceptionPayload::new("Cause", "cause"),
            TOO_NEW_VERSION,
        ))
        .with_context(with_version(ExceptionPayload::new("Context", "context"), 0));

    assert_nested_failure(
        payload.validate_version(),
        "cause",
        expected_mismatch(TOO_NEW_VERSION),
    );
}

/// Exception-group members are visited in index order.
#[rstest]
fn earlier_group_members_are_reported_first() {
    let payload = ExceptionPayload {
        exceptions: vec![
            ExceptionPayload::new("Member0", "m0"),
            with_version(ExceptionPayload::new("Member1", "m1"), 0),
            ExceptionPayload::new("Member2", "m2"),
            with_version(ExceptionPayload::new("Member3", "m3"), TOO_NEW_VERSION),
        ],
        ..ExceptionPayload::new("Group", "group")
    };

    assert_nested_failure(
        payload.validate_version(),
        "exceptions[1]",
        expected_mismatch(0),
    );
}

/// A chain one node longer than the depth limit still validates.
#[rstest]
fn a_chain_at_the_depth_limit_validates() {
    let payload = build_exception_chain(ChainKind::Cause, MAX_EXCEPTION_PAYLOAD_DEPTH + 1);

    payload
        .validate_version()
        .expect("the deepest payload sits exactly at the depth limit");
}

/// A chain past the depth limit reports the limit and the offending path.
#[rstest]
fn a_chain_past_the_depth_limit_reports_the_limit() {
    let payload = build_exception_chain(ChainKind::Cause, MAX_EXCEPTION_PAYLOAD_DEPTH + 2);

    let error = payload
        .validate_version()
        .expect_err("the chain is deeper than the limit");

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
                segments.iter().all(|segment| *segment == "cause"),
                "unexpected path: {path}"
            );
        }
        other => panic!("expected TreeTooDeep, got {other:?}"),
    }
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

/// A tree holding one node more than the limit reports the limit.
#[rstest]
fn a_tree_past_the_node_limit_reports_the_limit() {
    let error = group_with_nodes(MAX_EXCEPTION_PAYLOAD_NODES + 1)
        .validate_version()
        .expect_err("the tree exceeds the node limit");

    match error {
        SchemaVersionError::TreeTooLarge { max_nodes } => {
            assert_eq!(max_nodes, MAX_EXCEPTION_PAYLOAD_NODES);
        }
        other => panic!("expected TreeTooLarge, got {other:?}"),
    }
}
