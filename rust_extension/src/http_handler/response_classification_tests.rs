//! Unit tests for worker response classification.

use rstest::rstest;

use crate::http_handler::worker::{ResponseClass, classify_status};

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
