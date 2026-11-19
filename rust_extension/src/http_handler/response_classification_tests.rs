//! Unit tests for worker response classification.
//!
//! These are the HTTP worker's status-code rules. The worker decides whether a
//! response means the record was accepted or the peer refused it, and the retry
//! logic in the parent `http_handler` module depends on that decision. The
//! `status_classification` cases drive the decision table, one status per case,
//! and assert the `ResponseClass` each maps to. A connection failure never
//! reaches this table: it takes the worker's separate transport-error path, so
//! these tests cover status codes only. This is the unit boundary beneath the
//! integration tests in `http_handler::tests`.

use rstest::rstest;

use crate::http_handler::response::{ResponseClass, classify_status};

#[rstest]
#[case(200, ResponseClass::Success)]
#[case(201, ResponseClass::Success)]
#[case(204, ResponseClass::Success)]
#[case(400, ResponseClass::Permanent)]
#[case(401, ResponseClass::Permanent)]
#[case(403, ResponseClass::Permanent)]
#[case(404, ResponseClass::Permanent)]
#[case(429, ResponseClass::Retryable)]
#[case(500, ResponseClass::Retryable)]
#[case(502, ResponseClass::Retryable)]
#[case(503, ResponseClass::Retryable)]
fn status_classification(#[case] status: u16, #[case] expected: ResponseClass) {
    assert_eq!(classify_status(status), expected);
}
