//! Unit tests for how a request's headers and credentials are assembled.
//!
//! The wire-level shape of a request is covered by `request_shape_tests.rs`,
//! which needs a listener and a real attempt. What lives here is the part that
//! can be decided from the configuration alone: which value wins when auth and
//! a custom header name the same header, and that a duplicate never survives
//! into the map ureq appends from.

use std::collections::HashMap;
use std::time::Duration;

use proptest::prelude::*;
use rstest::rstest;

use super::*;

/// Scenario: an agent is built for a handler with distinct timeouts.
///
/// Invariant: the handler's connect timeout and write timeout reach ureq as
/// its connect and global timeouts, and every other behaviour-relevant setting
/// ureq 3 defaults differently from ureq 2 is restored explicitly. A change
/// that no loopback fixture notices - proxy discovery switching on, statuses
/// raised as errors, ureq following redirects itself - still fails here.
#[test]
fn the_agent_restores_the_ureq_2_settings() {
    let connect = Duration::from_secs(7);
    let global = Duration::from_secs(11);

    let agent = build_agent(connect, global);
    let config = agent.config();
    let timeouts = config.timeouts();

    assert_eq!(timeouts.connect, Some(connect));
    assert_eq!(timeouts.global, Some(global));
    assert!(!config.http_status_as_error());
    assert!(config.proxy().is_none());
    assert_eq!(
        config.max_redirects(),
        0,
        "the transport follows redirects, so ureq must return every 3xx response"
    );
    assert!(
        !config.max_redirects_will_error(),
        "ureq must return the 3xx rather than raise it"
    );
    assert_eq!(config.redirect_auth_headers(), RedirectAuthHeaders::Never);
}

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

/// Scenario: a header name, a header value, or a bearer token that cannot be
/// encoded as an HTTP header.
///
/// Invariant: the rejection never echoes the configured secret. Each case
/// reaches a different branch: the name is rejected before its value is
/// examined, the value is rejected on its own, and the bearer token is rejected
/// while rendering the `Authorization` value.
#[rstest]
#[case::invalid_name(AuthConfig::None, &[("bad header", "secret-value")])]
#[case::invalid_value(AuthConfig::None, &[("x-token", "secret-value\n")])]
#[case::invalid_bearer(
    AuthConfig::Bearer {
        token: "secret-value\n".to_owned(),
    },
    &[]
)]
fn a_rejected_header_or_credential_is_never_echoed(
    #[case] auth: AuthConfig,
    #[case] headers: &[(&str, &str)],
) {
    let config = config(
        HTTPMethod::POST,
        auth,
        SerializationFormat::UrlEncoded,
        headers,
    );

    let err = resolve_headers(&config).expect_err("the input is not a valid header");

    assert!(
        !err.contains("secret-value"),
        "the error must not echo a configured secret: {err}"
    );
}

/// Scenario: a text header - `Authorization` or `Host` - whose value is not
/// ASCII text, in several spellings of the name.
///
/// Invariant: the builder's validation rejects it. `HeaderValue::from_str`
/// admits any byte from `0x20` upwards, but the send path decodes these two
/// values as text, so a value such as `Bearer café` would build successfully
/// and then fail every attempt. The error must not echo the value.
#[rstest]
#[case::authorization("Authorization", "Bearer café")]
#[case::authorization_lowercase("authorization", "Bearer café")]
#[case::host("Host", "café.example")]
#[case::host_mixed_case("HoSt", "café.example")]
fn a_non_ascii_text_header_is_rejected(#[case] name: &str, #[case] value: &str) {
    let config = config(
        HTTPMethod::POST,
        AuthConfig::None,
        SerializationFormat::UrlEncoded,
        &[(name, value)],
    );

    let err = validate_request_headers(&config).expect_err("the value cannot be sent");

    assert!(
        !err.contains(value),
        "the error must not echo the configured value: {err}"
    );
}

/// Header names the send path reads as text, in spellings `HeaderMap` treats
/// as the same name.
///
/// `Host` and `Authorization` are rare in generated input, so a uniform string
/// strategy would almost never exercise the two rules that differ from plain
/// `HeaderValue` parsing.
fn text_header_name() -> impl Strategy<Value = String> {
    prop::sample::select(vec![
        "Authorization",
        "authorization",
        "AUTHORIZATION",
        "Host",
        "host",
        "HoSt",
    ])
    .prop_map(str::to_owned)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// Scenario: any header pair a caller can configure.
    ///
    /// Invariant: validation accepts exactly the pairs the send path accepts: a
    /// parsable name, a parsable value, and - for `Authorization` and `Host`,
    /// which the send path decodes as text - a value `HeaderValue::to_str`
    /// still accepts. The fixed names are mixed into the generator so the two
    /// text rules are exercised far more often than random generation alone
    /// would produce them; the deterministic cases in
    /// [`a_non_ascii_text_header_is_rejected`] cover each rule for certain.
    #[test]
    fn header_validation_matches_the_send_path(
        name in prop_oneof![3 => any::<String>(), 1 => text_header_name()],
        value in any::<String>(),
    ) {
        let config = config(
            HTTPMethod::POST,
            AuthConfig::None,
            SerializationFormat::UrlEncoded,
            &[(name.as_str(), value.as_str())],
        );
        let accepted = validate_request_headers(&config);

        let name_ok = HeaderName::from_bytes(name.as_bytes()).is_ok();
        let encoded = HeaderValue::from_str(&value).ok();
        let text_header = name.eq_ignore_ascii_case("authorization")
            || name.eq_ignore_ascii_case("host");
        let sendable = encoded.as_ref().is_some_and(|value| value.to_str().is_ok());
        prop_assert_eq!(
            accepted.is_ok(),
            name_ok && encoded.is_some() && (!text_header || sendable),
            "name={:?} value={:?}",
            name,
            value
        );
        if name_ok && encoded.is_none() {
            let err = accepted.expect_err("an unencodable value is rejected");
            prop_assert!(!err.contains(&value), "the error echoed the value");
        }
    }

    /// Scenario: any Basic or Bearer credential a caller can configure.
    ///
    /// Invariant: a Bearer token is accepted exactly when the rendered
    /// `Authorization` value is ASCII text, which is what the send path
    /// decodes; a Basic credential is always base64-encoded to ASCII. A
    /// rejection never echoes the token.
    #[test]
    fn credential_validation_matches_the_send_path(
        token in any::<String>(),
        username in any::<String>(),
        password in any::<String>(),
    ) {
        let bearer = config(
            HTTPMethod::POST,
            AuthConfig::Bearer { token: token.clone() },
            SerializationFormat::UrlEncoded,
            &[],
        );
        let accepted = validate_request_headers(&bearer);
        let rendered_is_text = format!("Bearer {token}")
            .bytes()
            .all(|byte| byte == b'\t' || (0x20..0x7f).contains(&byte));
        prop_assert_eq!(accepted.is_ok(), rendered_is_text, "token={:?}", token);
        if let Err(err) = accepted {
            prop_assert!(!token.is_empty() && !err.contains(&token));
        }

        let basic = config(
            HTTPMethod::POST,
            AuthConfig::Basic {
                username,
                password,
            },
            SerializationFormat::UrlEncoded,
            &[],
        );
        prop_assert!(
            validate_request_headers(&basic).is_ok(),
            "base64 keeps a Basic credential sendable"
        );
    }
}
