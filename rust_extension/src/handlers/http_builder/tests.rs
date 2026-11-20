//! Tests for the HTTP handler builder.

use rstest::rstest;

use super::*;

/// Scenario: a header name, a header value, or a bearer token that cannot be
/// encoded as an HTTP header.
///
/// Invariant: building the handler fails with a configuration error that does
/// not echo the configured secret. ureq 2 wrote header lines to the wire
/// verbatim; ureq 3 rejects them, so without this check every record would be
/// retried until the backoff deadline.
#[rstest]
#[case::invalid_name(&[("bad header", "secret-value")], None)]
#[case::invalid_value(&[("x-token", "secret-value\n")], None)]
#[case::invalid_bearer(&[], Some("secret-value\n"))]
fn rejects_an_unencodable_header_without_echoing_it(
    #[case] headers: &[(&str, &str)],
    #[case] bearer: Option<&str>,
) {
    let mut builder = HTTPHandlerBuilder::new().with_url("http://example.com/log");
    for (name, value) in headers {
        builder = builder.with_header(*name, *value);
    }
    if let Some(token) = bearer {
        builder = builder.with_bearer_token(token);
    }

    let Err(HandlerBuildError::InvalidConfig(message)) = builder.build_inner() else {
        panic!("an unencodable header must reject the configuration");
    };
    assert!(
        !message.contains("secret-value"),
        "the error must not echo a configured secret: {message}"
    );
}

#[test]
fn allows_an_encodable_custom_header() {
    HTTPHandlerBuilder::new()
        .with_url("http://example.com/log")
        .with_header("x-api-key", "custom-credential")
        .build_inner()
        .expect("an encodable header must build");
}

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

/// Scenario: the endpoint contains a character that must be percent-encoded,
/// such as the space in a log ingestion path.
///
/// Invariant: the endpoint is accepted, because the transport parses every
/// request target with `url::Url` exactly as ureq 2 did. The bytes on the wire
/// are asserted by `request_shape_tests/endpoint.rs`; this checks that the
/// builder does not reject what the send path can encode.
#[test]
fn accepts_an_endpoint_whose_path_needs_percent_encoding() {
    let builder = HTTPHandlerBuilder::new().with_url("http://example.com/log ingest");

    builder
        .validate_endpoint()
        .expect("a space is encodable by the send path");
}

/// Scenario: the endpoint is not one the transport can send.
///
/// Invariant: building fails with a configuration error naming the endpoint,
/// rather than spawning a worker that fails every attempt and retries each
/// record until the backoff deadline expires. The cases are a scheme other than
/// HTTP, a relative path, and a string that is not a URL.
#[rstest::rstest]
#[case::file("file:///var/log/records")]
#[case::relative("/log")]
#[case::unparseable("not a url")]
fn rejects_an_endpoint_the_transport_cannot_send(#[case] endpoint: &str) {
    let err = HTTPHandlerBuilder::new()
        .with_url(endpoint)
        .build_inner()
        .expect_err("a non-HTTP endpoint must reject the configuration");

    assert!(
        matches!(err, HandlerBuildError::InvalidConfig(ref message) if message.contains("endpoint")),
        "the error must identify the endpoint: {err:?}"
    );
}
