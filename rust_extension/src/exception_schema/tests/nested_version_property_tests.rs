//! Property tests for recursive schema version validation.
//!
//! A reference model walks a generated tree shape in the same depth-first
//! pre-order as the validator, so the reported path and inner error can be
//! compared with the first unsupported node the model finds. Bounded cases
//! pin both traversal limits at their exact boundaries.

use crate::exception_schema::{
    EXCEPTION_SCHEMA_VERSION, ExceptionPayload, MAX_EXCEPTION_PAYLOAD_DEPTH,
    MAX_EXCEPTION_PAYLOAD_NODES, MIN_EXCEPTION_SCHEMA_VERSION, SchemaVersionError, SchemaVersioned,
};
use proptest::prelude::*;
use proptest::sample::Index;

use super::deep_chain_tests::{ChainKind, build_exception_chain};
use super::nested_version_limit_tests::group_with_nodes;

/// A generated payload-tree shape.
#[derive(Clone, Debug)]
enum Shape {
    /// A payload with no nested payload.
    Leaf,
    /// A payload linked to a `cause` child.
    Cause(Box<Shape>),
    /// A payload linked to a `context` child.
    Context(Box<Shape>),
    /// A group payload holding exception-group members.
    Group(Vec<Shape>),
}

/// Strategy for bounded tree shapes.
fn shape_strategy() -> impl Strategy<Value = Shape> {
    let leaf = Just(Shape::Leaf);
    leaf.prop_recursive(4, 32, 4, |inner| {
        prop_oneof![
            inner
                .clone()
                .prop_map(|shape| Shape::Cause(Box::new(shape))),
            inner
                .clone()
                .prop_map(|shape| Shape::Context(Box::new(shape))),
            prop::collection::vec(inner, 1..4).prop_map(Shape::Group),
        ]
    })
}

/// Collect the dot-separated path of every node in depth-first pre-order.
fn preorder_paths(shape: &Shape) -> Vec<String> {
    fn walk(shape: &Shape, segments: &mut Vec<String>, paths: &mut Vec<String>) {
        paths.push(segments.join("."));
        match shape {
            Shape::Leaf => {}
            Shape::Cause(child) => {
                segments.push("cause".to_owned());
                walk(child, segments, paths);
                segments.pop();
            }
            Shape::Context(child) => {
                segments.push("context".to_owned());
                walk(child, segments, paths);
                segments.pop();
            }
            Shape::Group(children) => {
                for (index, child) in children.iter().enumerate() {
                    segments.push(format!("exceptions[{index}]"));
                    walk(child, segments, paths);
                    segments.pop();
                }
            }
        }
    }

    let mut paths = Vec::new();
    walk(shape, &mut Vec::new(), &mut paths);
    paths
}

/// Build the payload tree, giving the `target`-th pre-order node `version`.
fn build_payload(shape: &Shape, target: usize, version: u16) -> ExceptionPayload {
    /// Builds payload nodes in pre-order while counting their indices.
    struct Builder {
        /// Pre-order index of the next node to build.
        next: usize,
        /// Pre-order index of the node that receives `version`.
        target: usize,
        /// Schema version given to the target node.
        version: u16,
    }

    impl Builder {
        fn node(&mut self, shape: &Shape) -> ExceptionPayload {
            let index = self.next;
            self.next += 1;

            let mut payload = ExceptionPayload::new("Node", format!("node {index}"));
            if index == self.target {
                payload.schema_version = self.version;
            }
            match shape {
                Shape::Leaf => {}
                Shape::Cause(child) => payload.cause = Some(Box::new(self.node(child))),
                Shape::Context(child) => payload.context = Some(Box::new(self.node(child))),
                Shape::Group(children) => {
                    payload.exceptions = children.iter().map(|child| self.node(child)).collect();
                }
            }
            payload
        }
    }

    Builder {
        next: 0,
        target,
        version,
    }
    .node(shape)
}

/// The error the reference model expects for `version` at `path`.
fn expected_error(path: &str, version: u16) -> SchemaVersionError {
    let mismatch = if version > EXCEPTION_SCHEMA_VERSION {
        SchemaVersionError::VersionTooNew {
            found: version,
            max_supported: EXCEPTION_SCHEMA_VERSION,
        }
    } else {
        SchemaVersionError::VersionTooOld {
            found: version,
            min_supported: MIN_EXCEPTION_SCHEMA_VERSION,
        }
    };

    if path.is_empty() {
        mismatch
    } else {
        SchemaVersionError::Nested {
            path: path.to_owned(),
            source: Box::new(mismatch),
        }
    }
}

proptest! {
    /// The validator reports the first unsupported node in pre-order order.
    #[test]
    fn the_first_unsupported_node_is_reported(
        shape in shape_strategy(),
        target in any::<Index>(),
        version in prop_oneof![
            Just(0u16),
            Just(EXCEPTION_SCHEMA_VERSION + 1),
            Just(u16::MAX),
        ],
    ) {
        let paths = preorder_paths(&shape);
        let target = target.index(paths.len());
        let payload = build_payload(&shape, target, version);

        prop_assert_eq!(
            payload.validate_version(),
            Err(expected_error(&paths[target], version))
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4))]

    /// The depth limit accepts the node at the limit and rejects the next one.
    #[test]
    fn the_depth_limit_boundary_is_exact(extra in 0usize..3) {
        let nodes = MAX_EXCEPTION_PAYLOAD_DEPTH + 1 + extra;
        let payload = build_exception_chain(ChainKind::Cause, nodes);
        let outcome = payload.validate_version();

        if extra == 0 {
            prop_assert_eq!(outcome, Ok(()));
        } else {
            // Bind the predicate first: `matches!` braces would be parsed as a
            // format string by `prop_assert!`'s generated message.
            let is_too_deep =
                matches!(outcome, Err(SchemaVersionError::TreeTooDeep { .. }));
            prop_assert!(is_too_deep);
        }
    }

    /// The node limit accepts the tree at the limit and rejects the next node.
    #[test]
    fn the_node_limit_boundary_is_exact(extra in 0usize..3) {
        let nodes = MAX_EXCEPTION_PAYLOAD_NODES + extra;
        let payload = group_with_nodes(nodes);
        let outcome = payload.validate_version();

        if extra == 0 {
            prop_assert_eq!(outcome, Ok(()));
        } else {
            let is_too_large =
                matches!(outcome, Err(SchemaVersionError::TreeTooLarge { .. }));
            prop_assert!(is_too_large);
        }
    }
}
