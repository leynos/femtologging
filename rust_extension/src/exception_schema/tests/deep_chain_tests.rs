//! Tests for deep exception chain serialization.
//!
//! These tests verify that deeply nested cause and context chains serialize
//! correctly without stack overflow or performance degradation.

use crate::exception_schema::ExceptionPayload;
use rstest::rstest;

/// The kind of link used to build (and traverse) an exception chain.
///
/// Modelling the chain kind as an enum keeps an invalid chain kind
/// unrepresentable, so neither helper below needs a fallible catch-all arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChainKind {
    /// Every level is linked through `__cause__`.
    Cause,
    /// Every level is linked through `__context__`.
    Context,
    /// Levels alternate between cause and context links.
    Mixed,
}

/// Builds an exception chain of the specified kind and depth.
///
/// # Arguments
/// * `kind` - Which link type joins successive levels
/// * `depth` - Total number of exceptions in the chain
fn build_exception_chain(kind: ChainKind, depth: usize) -> ExceptionPayload {
    let mut current = ExceptionPayload::new("BaseError", "root");
    for i in 1..depth {
        current = match kind {
            ChainKind::Cause => {
                ExceptionPayload::new(format!("Error{i}"), format!("level {i}")).with_cause(current)
            }
            ChainKind::Context => {
                ExceptionPayload::new(format!("Error{i}"), format!("context level {i}"))
                    .with_context(current)
            }
            ChainKind::Mixed => {
                if i % 2 == 0 {
                    ExceptionPayload::new(format!("CauseError{i}"), format!("cause {i}"))
                        .with_cause(current)
                } else {
                    ExceptionPayload::new(format!("ContextError{i}"), format!("context {i}"))
                        .with_context(current)
                }
            }
        };
    }
    current
}

/// Counts the levels of a chain by following the links for `kind`.
///
/// Cause chains follow only cause links, context chains only context links, and
/// mixed chains follow whichever link exists at each level.
fn chain_depth(payload: &ExceptionPayload, kind: ChainKind) -> usize {
    let mut depth = 0;
    let mut node = Some(payload);
    while let Some(n) = node {
        depth += 1;
        node = match kind {
            ChainKind::Cause => n.cause.as_deref(),
            ChainKind::Context => n.context.as_deref(),
            ChainKind::Mixed => n.cause.as_deref().or(n.context.as_deref()),
        };
    }
    depth
}

/// Verifies that a payload has the expected chain depth by traversing links.
#[track_caller]
fn verify_chain_depth(payload: &ExceptionPayload, kind: ChainKind, expected_depth: usize) {
    let depth = chain_depth(payload, kind);
    assert_eq!(
        depth, expected_depth,
        "Expected chain depth {expected_depth}, found {depth}"
    );
}

#[rstest]
#[case(ChainKind::Cause, 10)]
#[case(ChainKind::Cause, 100)]
#[case(ChainKind::Context, 100)]
#[case(ChainKind::Mixed, 50)]
fn deep_chain_serializes(#[case] kind: ChainKind, #[case] depth: usize) {
    let payload = build_exception_chain(kind, depth);

    let json = serde_json::to_string(&payload).expect("serialize deep chain");
    let decoded: ExceptionPayload = serde_json::from_str(&json).expect("deserialize");

    verify_chain_depth(&decoded, kind, depth);
}

/// Regression guard: 100-level cause chain should serialize in linear time.
///
/// This test uses a generous 10-second threshold to avoid flakiness in CI
/// while still catching quadratic or exponential time regressions.
#[rstest]
#[ignore = "timing-sensitive; run manually or via heavy-tests workflow"]
fn deep_cause_chain_100_levels_timing() {
    let start = std::time::Instant::now();

    let payload = build_exception_chain(ChainKind::Cause, 100);

    let json = serde_json::to_string(&payload).expect("serialize deep chain");
    let _decoded: ExceptionPayload = serde_json::from_str(&json).expect("deserialize");

    let elapsed = start.elapsed();
    assert!(
        elapsed.as_secs() < 10,
        "Deep chain serialization took too long: {:?} (expected < 10s)",
        elapsed
    );
}
