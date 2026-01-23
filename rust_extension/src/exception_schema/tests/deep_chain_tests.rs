//! Tests for deep exception chain serialization.
//!
//! These tests verify that deeply nested cause and context chains serialize
//! correctly without stack overflow or performance degradation.

use crate::exception_schema::ExceptionPayload;
use rstest::rstest;

/// Builds an exception chain of the specified type and depth.
///
/// # Arguments
/// * `chain_type` - "cause", "context", or "mixed" (alternates cause/context)
/// * `depth` - Total number of exceptions in the chain
fn build_exception_chain(chain_type: &str, depth: usize) -> ExceptionPayload {
    let mut current = ExceptionPayload::new("BaseError", "root");
    for i in 1..depth {
        current = match chain_type {
            "cause" => {
                ExceptionPayload::new(format!("Error{i}"), format!("level {i}")).with_cause(current)
            }
            "context" => ExceptionPayload::new(format!("Error{i}"), format!("context level {i}"))
                .with_context(current),
            "mixed" => {
                if i % 2 == 0 {
                    ExceptionPayload::new(format!("CauseError{i}"), format!("cause {i}"))
                        .with_cause(current)
                } else {
                    ExceptionPayload::new(format!("ContextError{i}"), format!("context {i}"))
                        .with_context(current)
                }
            }
            _ => panic!("Unknown chain type: {chain_type}"),
        };
    }
    current
}

/// Verifies that a payload has the expected chain depth by traversing links.
///
/// For "cause" chains, follows only cause links.
/// For "context" chains, follows only context links.
/// For "mixed" chains, follows whichever link exists at each level.
fn verify_chain_depth(payload: &ExceptionPayload, chain_type: &str, expected_depth: usize) {
    let mut depth = 0;
    let mut node = Some(payload);
    while let Some(n) = node {
        depth += 1;
        node = match chain_type {
            "cause" => n.cause.as_deref(),
            "context" => n.context.as_deref(),
            "mixed" => n.cause.as_deref().or(n.context.as_deref()),
            _ => panic!("Unknown chain type: {chain_type}"),
        };
    }
    assert_eq!(
        depth, expected_depth,
        "Expected chain depth {expected_depth}, found {depth}"
    );
}

#[rstest]
#[case("cause", 10)]
#[case("cause", 100)]
#[case("context", 100)]
#[case("mixed", 50)]
fn deep_chain_serializes(#[case] chain_type: &str, #[case] depth: usize) {
    let payload = build_exception_chain(chain_type, depth);

    let json = serde_json::to_string(&payload).expect("serialize deep chain");
    let decoded: ExceptionPayload = serde_json::from_str(&json).expect("deserialize");

    verify_chain_depth(&decoded, chain_type, depth);
}

/// Regression guard: 100-level cause chain should serialize in linear time.
///
/// This test uses a generous 10-second threshold to avoid flakiness in CI
/// while still catching quadratic or exponential time regressions.
#[rstest]
#[ignore = "timing-sensitive; run manually or via heavy-tests workflow"]
fn deep_cause_chain_100_levels_timing() {
    let start = std::time::Instant::now();

    let payload = build_exception_chain("cause", 100);

    let json = serde_json::to_string(&payload).expect("serialize deep chain");
    let _decoded: ExceptionPayload = serde_json::from_str(&json).expect("deserialize");

    let elapsed = start.elapsed();
    assert!(
        elapsed.as_secs() < 10,
        "Deep chain serialization took too long: {:?} (expected < 10s)",
        elapsed
    );
}
