//! Tests for the HTTP handler builder.

use super::*;

#[test]
fn rejects_get_with_json_format() {
    let result = HTTPHandlerBuilder::new()
        .with_url("http://example.com/log")
        .with_method(HTTPMethod::GET)
        .with_json_format()
        .build_inner();

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(err, HandlerBuildError::InvalidConfig(_)));
    if let HandlerBuildError::InvalidConfig(msg) = err {
        assert!(msg.contains("JSON"));
        assert!(msg.contains("GET"));
    }
}

#[test]
fn allows_post_with_json_format() {
    let result = HTTPHandlerBuilder::new()
        .with_url("http://example.com/log")
        .with_method(HTTPMethod::POST)
        .with_json_format()
        .build_inner();

    assert!(result.is_ok());
}

#[test]
fn allows_get_with_url_encoded_format() {
    let result = HTTPHandlerBuilder::new()
        .with_url("http://example.com/log")
        .with_method(HTTPMethod::GET)
        .build_inner();

    assert!(result.is_ok());
}
