//! Unit tests for the HTTP worker's own helpers.
//!
//! Response classification has its own file, `response_classification_tests.rs`,
//! and is not repeated here: what is left in the worker is the authentication
//! encoding that no other module owns.

use super::*;

#[test]
fn base64_encode_basic() {
    assert_eq!(base64_encode(b"user:pass"), "dXNlcjpwYXNz");
    assert_eq!(base64_encode(b"a"), "YQ==");
    assert_eq!(base64_encode(b"ab"), "YWI=");
    assert_eq!(base64_encode(b"abc"), "YWJj");
}
