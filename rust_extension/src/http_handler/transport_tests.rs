//! Unit tests for how a request's headers and credentials are assembled.
//!
//! The wire-level shape of a request is covered by `request_shape_tests.rs`,
//! which needs a listener and a real attempt. What lives here is the part that
//! can be decided from the configuration alone: which value wins when auth and
//! a custom header name the same header, and that a duplicate never survives
//! into the map ureq appends from.

use std::collections::HashMap;

use super::*;

/// Build a configuration whose only interesting parts are the given values.
fn config(
    method: HTTPMethod,
    auth: AuthConfig,
    format: SerializationFormat,
    headers: &[(&str, &str)],
) -> HTTPHandlerConfig {
    HTTPHandlerConfig {
        method,
        auth,
        format,
        headers: headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect::<HashMap<_, _>>(),
        ..Default::default()
    }
}

#[test]
fn base64_encode_basic() {
    assert_eq!(base64_encode(b"user:pass"), "dXNlcjpwYXNz");
    assert_eq!(base64_encode(b"a"), "YQ==");
    assert_eq!(base64_encode(b"ab"), "YWI=");
    assert_eq!(base64_encode(b"abc"), "YWJj");
}

#[test]
fn basic_auth_encodes_the_credentials_once() {
    let config = config(
        HTTPMethod::POST,
        AuthConfig::Basic {
            username: "user".to_owned(),
            password: "pass".to_owned(),
        },
        SerializationFormat::UrlEncoded,
        &[],
    );

    let headers = resolve_headers(&config).expect("valid header values");

    assert_eq!(
        headers
            .get(AUTHORIZATION)
            .expect("basic auth sets Authorization"),
        "Basic dXNlcjpwYXNz"
    );
}

#[test]
fn a_custom_authorization_header_replaces_configured_auth() {
    // The header name is spelled differently on purpose: `HeaderMap` is
    // case-insensitive, and this is the spelling the old overwriting code had
    // to match.
    let config = config(
        HTTPMethod::POST,
        AuthConfig::Bearer {
            token: "configured".to_owned(),
        },
        SerializationFormat::UrlEncoded,
        &[("authorization", "Bearer custom")],
    );

    let headers = resolve_headers(&config).expect("valid header values");

    assert_eq!(
        headers.get(AUTHORIZATION).expect("one Authorization"),
        "Bearer custom"
    );
    assert_eq!(
        headers.get_all(AUTHORIZATION).iter().count(),
        1,
        "ureq appends per call, so the map must already be collapsed to one value"
    );
}

#[test]
fn the_format_content_type_replaces_a_custom_header() {
    // Precedence is the one the ureq 2 code produced: the format's content type
    // was set last, so it overwrote a custom header of the same name.
    let config = config(
        HTTPMethod::POST,
        AuthConfig::None,
        SerializationFormat::UrlEncoded,
        &[("Content-Type", "text/plain")],
    );

    let headers = resolve_headers(&config).expect("valid header values");

    assert_eq!(
        headers.get(CONTENT_TYPE).expect("one Content-Type"),
        "application/x-www-form-urlencoded"
    );
    assert_eq!(headers.get_all(CONTENT_TYPE).iter().count(), 1);
}

#[test]
fn a_get_request_carries_no_content_type() {
    let config = config(
        HTTPMethod::GET,
        AuthConfig::None,
        SerializationFormat::UrlEncoded,
        &[],
    );

    let headers = resolve_headers(&config).expect("valid header values");

    assert!(headers.get(CONTENT_TYPE).is_none());
}

#[test]
fn an_invalid_header_name_is_rejected_without_echoing_its_value() {
    let config = config(
        HTTPMethod::POST,
        AuthConfig::None,
        SerializationFormat::UrlEncoded,
        &[("bad header", "secret-value")],
    );

    let err = resolve_headers(&config).expect_err("a space is not valid in a header name");

    assert!(
        !err.contains("secret-value"),
        "the error must not echo a configured header value: {err}"
    );
}
