//! Send-path transport labels asserted through the production `execute` path.
//!
//! The classification vocabulary has its own unit table in `transport/failure`,
//! and the dependency's own errors are covered there. What this file adds is
//! the other half: labels the transport itself decides, driven through
//! `execute` exactly as the worker drives it. A label that is renamed, removed,
//! or accidentally replaced by dependency text fails here rather than only in a
//! list of accepted strings.

use std::io;
use std::net::TcpListener;
use std::time::Duration;

use rstest::rstest;

use crate::http_handler::config::{AuthConfig, HTTPMethod};

use super::redirects::{execute_redirect_chain, spawn_raw_response};
use super::*;

/// Scenario: the configured endpoint cannot be parsed at send time.
///
/// Invariant: the attempt reports the fixed `invalid URL` classification.
/// `HTTPHandlerBuilder` rejects such an endpoint when the handler is built, so
/// this path is reached only by a handler constructed directly from
/// `HTTPHandlerConfig`; the label is what that caller sees.
#[test]
fn an_unparseable_endpoint_is_reported_as_an_invalid_url() {
    let config = post_config("http://[::1/log".to_owned(), AuthConfig::None, &[]);

    let err = execute_attempt(
        &config,
        &test_agent(),
        "msg=bad-url",
        Duration::from_secs(1),
    )
    .expect_err("an endpoint that cannot be parsed must not be sent");

    assert_eq!(err, "transport error (invalid URL)");
}

/// Scenario: the attempt's budget has already been spent before its first hop.
///
/// Invariant: the attempt reports the fixed `attempt ran out of time`
/// classification. A zero budget cannot be represented by a positive remaining
/// allowance, so the loop stops before sending and the record follows the
/// normal retry policy under that label.
#[test]
fn a_spent_budget_is_reported_before_any_request() {
    let config = post_config("http://127.0.0.1:9/log".to_owned(), AuthConfig::None, &[]);

    let err = execute_attempt(&config, &test_agent(), "msg=late", Duration::ZERO)
        .expect_err("a spent budget must not send a request");

    assert_eq!(err, "transport error (attempt ran out of time)");
}

/// Scenario: a GET attempt against an endpoint whose host cannot resolve.
///
/// Invariant: the failure travels through the dependency classification rather
/// than its formatted text, so the retry warning sees a fixed label and no
/// endpoint. `.invalid` is reserved by RFC 2606 and never resolves, which keeps
/// the scenario independent of the host's resolver configuration. ureq reports
/// the resolver's failure as `Io`, so the label is the I/O classification; the
/// dedicated `DNS lookup failed` arm covers a connector that reports
/// `HostNotFound`.
#[test]
fn an_unresolvable_host_is_reported_without_the_endpoint() {
    let config = HTTPHandlerConfig {
        url: "http://collector.invalid/log".to_owned(),
        method: HTTPMethod::GET,
        ..Default::default()
    };

    let err = execute_attempt(&config, &test_agent(), "msg=dns", Duration::from_secs(5))
        .expect_err("a reserved host must not resolve");

    assert_eq!(err, "transport error (I/O error)");
    assert!(
        !err.contains("collector.invalid"),
        "the classification must not repeat the endpoint: {err}"
    );
}

/// Scenario: a chain answers with one redirect more than the limit.
///
/// Invariant: the limit reaches the worker's retry warning as the fixed
/// `reached max redirects` label. The redirect unit tests pin `next_hop`'s own
/// decision; this pins what the caller receives from `execute`.
#[rstest]
fn the_chain_limit_label_reaches_the_caller(tcp_listener: io::Result<TcpListener>) {
    let err = execute_redirect_chain(tcp_listener.expect("bind ephemeral listener"), 5)
        .expect_err("the sixth response exceeds the limit");

    assert_eq!(err, "reached max redirects");
}

/// Scenario: a collector answers with a `Location` that cannot be resolved.
///
/// Invariant: the failure reaches the caller as the fixed
/// `invalid redirect location` label, so the server-supplied value never
/// becomes part of the diagnostic.
#[rstest]
fn an_unresolvable_location_label_reaches_the_caller(tcp_listener: io::Result<TcpListener>) {
    let response = b"HTTP/1.1 302 Found\r\nLocation: http://[::1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec();
    let (addr, requests) =
        spawn_raw_response(tcp_listener.expect("bind ephemeral listener"), response)
            .expect("spawn raw response server");
    let config = post_config(format!("http://{addr}/log"), AuthConfig::None, &[]);

    let err = execute(&config, &test_agent(), "msg=redirect")
        .expect_err("a Location the parser rejects is a transport failure");

    requests
        .recv_timeout(Duration::from_secs(5))
        .expect("the original POST is sent once");
    assert_eq!(err, "invalid redirect location");
}
