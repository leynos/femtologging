//! Unit tests for HTTP worker status classification and encoding helpers.

use super::*;

#[test]
fn classify_2xx_as_success() {
    assert_eq!(classify_status(200), ResponseClass::Success);
    assert_eq!(classify_status(201), ResponseClass::Success);
    assert_eq!(classify_status(204), ResponseClass::Success);
}

#[test]
fn classify_5xx_as_retryable() {
    assert_eq!(classify_status(500), ResponseClass::Retryable);
    assert_eq!(classify_status(502), ResponseClass::Retryable);
    assert_eq!(classify_status(503), ResponseClass::Retryable);
}

#[test]
fn classify_429_as_retryable() {
    assert_eq!(classify_status(429), ResponseClass::Retryable);
}

#[test]
fn classify_4xx_as_permanent() {
    assert_eq!(classify_status(400), ResponseClass::Permanent);
    assert_eq!(classify_status(401), ResponseClass::Permanent);
    assert_eq!(classify_status(403), ResponseClass::Permanent);
    assert_eq!(classify_status(404), ResponseClass::Permanent);
}

#[test]
fn base64_encode_basic() {
    assert_eq!(base64_encode(b"user:pass"), "dXNlcjpwYXNz");
    assert_eq!(base64_encode(b"a"), "YQ==");
    assert_eq!(base64_encode(b"ab"), "YWI=");
    assert_eq!(base64_encode(b"abc"), "YWJj");
}
