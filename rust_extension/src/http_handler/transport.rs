//! One HTTP attempt: how its agent is configured, its headers resolved, and
//! its response classified.
//!
//! Split out of `worker.rs`, which owns the clock - the retry deadline and the
//! shutdown budget - rather than the shape of a request. The settings here
//! preserve the behaviour the handler had on ureq 2, where ureq 3's defaults
//! differ: the trust store, proxy discovery, redirect limits, redirect
//! credential stripping, and how status codes are reported.

use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD as BASE64_STANDARD};
use ureq::{
    Agent, Body, RequestBuilder,
    config::RedirectAuthHeaders,
    http::{
        HeaderMap, HeaderName, HeaderValue, Response,
        header::{AUTHORIZATION, CONTENT_TYPE},
    },
    tls::{RootCerts, TlsConfig, TlsProvider},
};

use super::{
    config::{AuthConfig, HTTPHandlerConfig, HTTPMethod, SerializationFormat},
    response::{ResponseClass, classify_status},
};

/// Redirect limit retained from ureq 2.12.1, whose agent default was five.
const MAX_REDIRECTS: u32 = 5;

/// A response whose body the worker deliberately never reads.
pub(super) type HttpResponse = Response<Body>;

/// Build an agent from the handler's timeouts and an explicit TLS trust choice.
///
/// `TlsProvider::NativeTls` with `RootCerts::PlatformVerifier` preserves the
/// pre-migration behaviour: native-tls validates against the host's trust
/// store. ureq 3 otherwise defaults to Rustls with bundled WebPKI roots, which
/// would silently change which certificates are trusted.
///
/// Every other behaviour-relevant ureq 3 default is set to its ureq 2
/// equivalent here: status codes are classified by the caller rather than
/// raised as errors, proxy discovery stays off, and redirects keep ureq 2's
/// five-hop limit, refusal to exceed it, and stripping of credentials.
pub(super) fn build_agent(
    connect_timeout: Duration,
    global_timeout: Duration,
    root_certs: RootCerts,
) -> Agent {
    let tls_config = TlsConfig::builder()
        .provider(TlsProvider::NativeTls)
        .root_certs(root_certs)
        .build();
    let config = Agent::config_builder()
        .timeout_connect(Some(connect_timeout))
        .timeout_global(Some(global_timeout))
        .http_status_as_error(false)
        .proxy(None)
        .max_redirects(MAX_REDIRECTS)
        .max_redirects_will_error(true)
        .redirect_auth_headers(RedirectAuthHeaders::Never)
        .tls_config(tls_config)
        .build();
    Agent::new_with_config(config)
}

/// Send one payload and classify the response.
///
/// A `StatusCode` error is classified like the matching success response, so a
/// change to ureq's `http_status_as_error` setting cannot silently turn a
/// server verdict into a transport failure.
pub(super) fn execute(
    config: &HTTPHandlerConfig,
    agent: &Agent,
    payload: &str,
) -> Result<ResponseClass, String> {
    let headers = resolve_headers(config)?;
    // Note: GET+JSON combination is rejected at build time by HTTPHandlerBuilder.
    let sent = match config.method {
        HTTPMethod::GET => send_get(config, agent, &headers, payload),
        HTTPMethod::POST => send_post(config, agent, &headers, payload),
    };

    match sent {
        Ok(response) => Ok(classify_status(response.status().as_u16())),
        Err(err) => match *err {
            ureq::Error::StatusCode(code) => Ok(classify_status(code)),
            other => Err(other.to_string()),
        },
    }
}

/// Build and send a GET request; the payload follows any existing query data.
fn send_get(
    config: &HTTPHandlerConfig,
    agent: &Agent,
    headers: &HeaderMap,
    payload: &str,
) -> Result<HttpResponse, Box<ureq::Error>> {
    let url = if config.url.contains('?') {
        format!("{}&{}", config.url, payload)
    } else {
        format!("{}?{}", config.url, payload)
    };
    apply_headers(agent.get(&url), headers)
        .call()
        .map_err(Box::new)
}

/// Build and send a POST request; the resolved headers carry the content type.
fn send_post(
    config: &HTTPHandlerConfig,
    agent: &Agent,
    headers: &HeaderMap,
    payload: &str,
) -> Result<HttpResponse, Box<ureq::Error>> {
    apply_headers(agent.post(config.url.as_str()), headers)
        .send(payload)
        .map_err(Box::new)
}

/// Apply each resolved header exactly once to a request builder.
///
/// ureq 3's `header` appends rather than replaces, so the caller must have
/// already collapsed duplicates into one case-insensitive map.
fn apply_headers<Any>(
    mut request: RequestBuilder<Any>,
    headers: &HeaderMap,
) -> RequestBuilder<Any> {
    for (name, value) in headers {
        request = request.header(name.clone(), value.clone());
    }
    request
}

/// Collapse auth, custom headers, and the POST content type into one map.
///
/// ureq 3 appends a header per call rather than replacing it, so a request
/// could otherwise carry two `Authorization` or `Content-Type` values. The
/// precedence is the one the ureq 2 code produced by overwriting: configured
/// authentication first, then any custom header of the same name, then the
/// content type implied by the serialization format for a POST body.
///
/// Error messages never include a header value, because one of them is a
/// credential.
///
/// A rejection is returned as an ordinary transport error, not a distinct
/// permanent verdict. ureq 3 validates header names and values that ureq 2
/// wrote to the wire verbatim, so this failure is new; reporting it as a
/// transport error keeps the worker's retry policy, which this migration must
/// not change. [`validate_request_headers`] catches the same rejection for a
/// handler built through `HTTPHandlerBuilder`, so only a handler constructed
/// directly from [`HTTPHandlerConfig`] can reach this path at send time.
fn resolve_headers(config: &HTTPHandlerConfig) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    if let Some(value) = auth_header_value(&config.auth)? {
        headers.insert(AUTHORIZATION, value);
    }
    for (name, value) in &config.headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| format!("invalid HTTP header name {name:?}"))?;
        let value = HeaderValue::from_str(value)
            .map_err(|_| format!("invalid value for HTTP header {name}"))?;
        headers.insert(name, value);
    }
    if config.method == HTTPMethod::POST {
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static(content_type(&config.format)),
        );
    }
    Ok(headers)
}

/// Reject a configuration whose headers or credentials cannot be encoded.
///
/// ureq 3 validates header names and values that ureq 2 wrote to the wire
/// verbatim, so a handler built with an unencodable header would otherwise
/// fail every attempt at send time and retry each record until the backoff
/// deadline. `HTTPHandlerBuilder` calls this before it spawns a worker, so the
/// defect is reported once, as a configuration error, instead. The error never
/// includes a header value, because one of them is a credential.
pub(crate) fn validate_request_headers(config: &HTTPHandlerConfig) -> Result<(), String> {
    resolve_headers(config).map(|_| ())
}

/// Render the configured authentication as an `Authorization` value.
fn auth_header_value(auth: &AuthConfig) -> Result<Option<HeaderValue>, String> {
    let rendered = match auth {
        AuthConfig::None => return Ok(None),
        AuthConfig::Basic { username, password } => {
            let credentials = format!("{username}:{password}");
            format!("Basic {}", base64_encode(credentials.as_bytes()))
        }
        AuthConfig::Bearer { token } => format!("Bearer {token}"),
    };
    HeaderValue::from_str(&rendered).map(Some).map_err(|_| {
        "authentication credentials cannot be encoded as an HTTP header value".to_owned()
    })
}

/// The content type implied by the configured serialization format.
fn content_type(format: &SerializationFormat) -> &'static str {
    match format {
        SerializationFormat::UrlEncoded => "application/x-www-form-urlencoded",
        SerializationFormat::Json => "application/json",
    }
}

/// Base64-encode a byte slice for Basic auth.
fn base64_encode(input: &[u8]) -> String {
    BASE64_STANDARD.encode(input)
}

/// Send one payload with an agent trusting `root_certs`.
///
/// Only the TLS tests use this. It drives the production request path against a
/// throwaway certificate authority without changing the host's trust store or
/// reading the environment. It carries the same non-Windows condition as the
/// module that calls it, so a Windows test build does not compile an unused
/// function.
#[cfg(all(test, not(target_os = "windows")))]
pub(super) fn execute_with_roots(
    config: &HTTPHandlerConfig,
    root_certs: RootCerts,
    payload: &str,
) -> Result<ResponseClass, String> {
    let agent = build_agent(config.connect_timeout, config.write_timeout, root_certs);
    execute(config, &agent, payload)
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
