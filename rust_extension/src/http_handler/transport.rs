//! One HTTP attempt: how its agent is configured, its headers resolved, its
//! redirect chain followed, and its response classified.
//!
//! Split out of `worker.rs`, which owns the clock - the retry deadline and the
//! shutdown budget - rather than the shape of a request. That budget reaches a
//! chain as one deadline, so following a redirect cannot extend an attempt.
//! The settings here preserve the behaviour the handler had on ureq 2, where
//! ureq 3's defaults differ: the TLS provider and its roots, proxy discovery,
//! redirect rules and credential stripping, and how status codes are reported.

use std::time::{Duration, Instant};

use base64::{Engine, engine::general_purpose::STANDARD as BASE64_STANDARD};
use ureq::{
    Agent, Body, RequestBuilder,
    config::RedirectAuthHeaders,
    http::{
        HeaderMap, HeaderName, HeaderValue, Response,
        header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, HOST, LOCATION},
    },
    tls::{RootCerts, TlsConfig, TlsProvider},
};
use url::Url;

use super::{
    config::{AuthConfig, HTTPHandlerConfig, HTTPMethod, SerializationFormat},
    redirect::{self, NextHop},
    response::{ResponseClass, classify_status},
};

/// A response whose body the worker deliberately never reads.
pub(super) type HttpResponse = Response<Body>;

/// Build an agent for a production attempt against the bundled WebPKI roots.
///
/// The trust choice is fixed here rather than at each call site, so a future
/// caller cannot select a weaker root by accident. Only the TLS tests vary it,
/// through the test-only `execute_with_roots`.
pub(super) fn build_agent(connect_timeout: Duration, global_timeout: Duration) -> Agent {
    build_agent_with_roots(connect_timeout, global_timeout, RootCerts::WebPki)
}

/// Build an agent from the handler's timeouts and an explicit TLS trust choice.
///
/// `TlsProvider::Rustls` with `RootCerts::WebPki` reproduces what ureq 2
/// actually used. Enabling ureq 2's `native-tls` feature only made a native-tls
/// connector available for an `AgentBuilder` to select; the handler never
/// selected one, so the default Rustls connector validated every request
/// against the WebPKI roots compiled into the binary. Naming both here keeps
/// that policy even if ureq changes its own defaults.
///
/// Every other behaviour-relevant ureq 3 default is set to its ureq 2
/// equivalent here: status codes are classified by the caller rather than
/// raised as errors, proxy discovery stays off, and redirect following is
/// disabled so that [`redirect::next_hop`] applies ureq 2's rules instead. The
/// `RedirectAuthHeaders` setting is kept because it also states the policy the
/// chain applies: a redirect never replays a credential.
fn build_agent_with_roots(
    connect_timeout: Duration,
    global_timeout: Duration,
    root_certs: RootCerts,
) -> Agent {
    let tls_config = TlsConfig::builder()
        .provider(TlsProvider::Rustls)
        .root_certs(root_certs)
        .build();
    let config = Agent::config_builder()
        .timeout_connect(Some(connect_timeout))
        .timeout_global(Some(global_timeout))
        .http_status_as_error(false)
        .proxy(None)
        // ureq 3 follows any 3xx with a `Location`, including statuses ureq 2
        // returned, and errors instead of returning a redirect it will not
        // replay. With no redirects followed, ureq hands back the 3xx response
        // and `execute` applies the ureq 2 rules to it.
        .max_redirects(0)
        .max_redirects_will_error(false)
        .redirect_auth_headers(RedirectAuthHeaders::Never)
        .tls_config(tls_config)
        .build();
    Agent::new_with_config(config)
}

/// One request in a redirect chain.
struct Hop<'a> {
    /// Absolute target, parsed with the parser ureq 2 used for every request.
    url: Url,
    /// Method this request sends.
    method: HTTPMethod,
    /// Payload this request carries; only the first request carries one.
    body: Option<&'a str>,
}

/// Send one payload and classify the final response.
///
/// A 3xx response is followed while [`redirect::next_hop`] allows it; anything
/// the chain refuses is classified by its status, which is what the ureq 2
/// worker did with a redirect response it received. A `StatusCode` error is
/// classified like the matching success response, so a change to ureq's
/// `http_status_as_error` setting cannot silently turn a server verdict into a
/// transport failure.
///
/// `budget` is the whole attempt's allowance, not one hop's. ureq 3's global
/// timeout covers the call, redirects included; because the chain is walked
/// here instead, each hop is given the time that is left rather than a fresh
/// budget. Five hops would otherwise occupy five times the timeout, which
/// during shutdown would keep the worker past the deadline the handler is
/// waiting on.
pub(super) fn execute(
    config: &HTTPHandlerConfig,
    agent: &Agent,
    payload: &str,
    budget: Duration,
) -> Result<ResponseClass, String> {
    let mut headers = resolve_headers(config)?;
    let mut hop = first_hop(config, payload)?;
    let mut responses_seen = 1;
    // `None` only if the budget is too large for the platform's clock to
    // represent, where the agent's own timeout is the only bound available.
    let deadline = Instant::now().checked_add(budget);
    loop {
        let remaining = remaining_before_hop(deadline)?;
        let response = match send_hop(agent, &hop, &headers, remaining) {
            Ok(response) => response,
            Err(err) => return classify_error(*err),
        };
        let status = response.status().as_u16();
        if !(300..=399).contains(&status) {
            return Ok(classify_status(status));
        }
        match redirect::next_hop(redirect::RedirectAttempt {
            base: &hop.url,
            method: &hop.method,
            status,
            location: response.headers().get(LOCATION),
            responses_seen,
        }) {
            NextHop::Follow { url, method } => {
                headers = redirected_headers(&headers);
                hop = Hop {
                    url,
                    method,
                    body: None,
                };
                responses_seen += 1;
            }
            NextHop::Refuse => return Ok(classify_status(status)),
            NextHop::Failed(message) => return Err(message),
        }
    }
}

/// Build the first request of an attempt.
///
/// ureq 2 parsed every request URL with `url::Url`, which percent-encodes
/// characters such as a space in the path; ureq 3 parses an `http::Uri` and
/// would reject the raw character at send time, turning every attempt into a
/// retryable transport error and dropping the record once the backoff deadline
/// expired. Parsing here keeps the accepted endpoints and the bytes on the wire
/// unchanged. A GET payload is appended before the parse, because ureq 2
/// appended it to the configured endpoint before ureq parsed the result.
fn first_hop<'a>(config: &HTTPHandlerConfig, payload: &'a str) -> Result<Hop<'a>, String> {
    // Note: GET+JSON combination is rejected at build time by HTTPHandlerBuilder.
    let (target, body) = match config.method {
        HTTPMethod::GET => {
            let separator = if config.url.contains('?') { '&' } else { '?' };
            (format!("{}{separator}{payload}", config.url), None)
        }
        HTTPMethod::POST => (config.url.clone(), Some(payload)),
    };
    let url = Url::parse(&target).map_err(|err| format!("invalid HTTP endpoint: {err}"))?;
    Ok(Hop {
        url,
        method: config.method.clone(),
        body,
    })
}

/// Send one request and return the response whose body is never read.
///
/// A followed redirect never keeps a body-bearing method, so the payload is
/// present only on the first request of a POST attempt.
fn send_hop(
    agent: &Agent,
    hop: &Hop<'_>,
    headers: &HeaderMap,
    remaining: Option<Duration>,
) -> Result<HttpResponse, Box<ureq::Error>> {
    let target = hop.url.as_str();
    match hop.method {
        HTTPMethod::POST => apply_headers(apply_budget(agent.post(target), remaining), headers)
            .send(hop.body.unwrap_or_default())
            .map_err(Box::new),
        HTTPMethod::GET => apply_headers(apply_budget(agent.get(target), remaining), headers)
            .call()
            .map_err(Box::new),
    }
}

/// The time an attempt has left, or an error once its budget is spent.
///
/// The error is a transport failure rather than a verdict, which is how ureq
/// reported an expired global timeout: the record is retried outside a drain
/// and counted as a shutdown drop inside one, because the worker refuses to
/// retry while shutting down.
fn remaining_before_hop(deadline: Option<Instant>) -> Result<Option<Duration>, String> {
    let Some(deadline) = deadline else {
        return Ok(None);
    };
    match deadline.checked_duration_since(Instant::now()) {
        Some(remaining) if !remaining.is_zero() => Ok(Some(remaining)),
        _ => Err("the attempt ran out of time before the next request could be sent".to_owned()),
    }
}

/// Narrow a request's global timeout to the time its attempt has left.
///
/// The override is per request, so the pooled agent - and the connections it
/// holds - is kept for the whole chain.
fn apply_budget<State>(
    request: RequestBuilder<State>,
    remaining: Option<Duration>,
) -> RequestBuilder<State> {
    match remaining {
        Some(remaining) => request.config().timeout_global(Some(remaining)).build(),
        None => request,
    }
}

/// Classify a transport error, preserving the verdict ureq 2 produced.
///
/// ureq raises a status as an error only when configured to; the agent sets
/// `http_status_as_error(false)`, so a raised status can only come from an
/// agent a caller built itself. Classifying it like the matching response keeps
/// that path equivalent. Every other error is a transport failure, including a
/// redirect that exceeded the chain limit: ureq 2 raised that one as an error
/// too, so it is not a verdict.
fn classify_error(err: ureq::Error) -> Result<ResponseClass, String> {
    match err {
        ureq::Error::StatusCode(code) => Ok(classify_status(code)),
        other => Err(other.to_string()),
    }
}

/// Drop the headers ureq 2 removed from a followed redirect.
///
/// The agent is configured with `RedirectAuthHeaders::Never`, which is ureq 2's
/// default: the `Authorization` header is never replayed at the redirect
/// target. ureq 2 removed a `Cookie` header for the same reason, and ureq 3
/// would otherwise forward one configured as a custom header.
fn redirected_headers(headers: &HeaderMap) -> HeaderMap {
    let mut forwarded = headers.clone();
    forwarded.remove(AUTHORIZATION);
    forwarded.remove(COOKIE);
    forwarded
}

/// Apply each resolved header exactly once to a request builder.
///
/// ureq 3's `header` appends rather than replaces, so the caller must have
/// already collapsed duplicates into one case-insensitive map.
fn apply_headers<State>(
    mut request: RequestBuilder<State>,
    headers: &HeaderMap,
) -> RequestBuilder<State> {
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
///
/// Two of the checks are stricter than `HeaderValue::from_str`. That constructor
/// accepts any byte from `0x20` upwards, while the send path decodes the
/// `Authorization` and `Host` values as text: ureq 3 rejects a value it cannot
/// read there, which would again turn every attempt into a retryable transport
/// error. The effective value is checked *after* header precedence, so a custom
/// `Authorization` that replaces the configured credentials is the one that
/// must be sendable.
pub(crate) fn validate_request_headers(config: &HTTPHandlerConfig) -> Result<(), String> {
    let headers = resolve_headers(config)?;
    for name in [AUTHORIZATION, HOST] {
        if let Some(value) = headers.get(&name) {
            value.to_str().map_err(|_| {
                format!("the {name} header cannot be sent: its value is not ASCII text")
            })?;
        }
    }
    Ok(())
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
    let agent = build_agent_with_roots(config.connect_timeout, config.write_timeout, root_certs);
    execute(config, &agent, payload, config.write_timeout)
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
