//! Where a configured endpoint's bytes end up on the wire.
//!
//! ureq 2 parsed every request URL with `url::Url`, so a character such as a
//! space in the path was percent-encoded before it was sent. ureq 3 parses an
//! `http::Uri` and rejects the raw character, which would turn every attempt
//! into a retryable transport error and drop the record once the backoff
//! deadline expired. These tests assert what a collector receives, not which
//! parser produced it.

use std::net::TcpListener;
use std::time::Duration;

use rstest::rstest;

use super::{spawn_mock_server, test_agent};
use crate::http_handler::config::{HTTPHandlerConfig, HTTPMethod};
use crate::http_handler::response::ResponseClass;
use crate::http_handler::transport::execute;

/// Run one attempt against a fresh listener and return the request it received.
fn exchanged(method: HTTPMethod, url: String) -> (ResponseClass, String) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral listener");
    let (addr, requests) = spawn_mock_server(listener, 200).expect("spawn mock server");
    let config = HTTPHandlerConfig {
        url: url.replace("{addr}", &addr.to_string()),
        method,
        ..Default::default()
    };

    let class = execute(&config, &test_agent(), "msg=encoded").expect("the endpoint is sendable");
    let request = requests
        .recv_timeout(Duration::from_secs(5))
        .expect("the request reaches the listener");
    (class, request.path)
}

/// Scenario: an endpoint whose path contains a character ureq 3 would reject.
///
/// Invariant: the collector receives the percent-encoded form ureq 2 sent, and
/// the attempt succeeds. Removing the parse in the transport makes the request
/// unbuildable, so this fails rather than silently retrying.
#[rstest]
#[case::space_in_path("/log ingest", "/log%20ingest")]
#[case::already_encoded("/log%20ingest", "/log%20ingest")]
fn a_path_is_percent_encoded_the_way_ureq_two_did(#[case] path: &str, #[case] expected: &str) {
    let (class, sent) = exchanged(HTTPMethod::POST, format!("http://{{addr}}{path}"));

    assert_eq!(class, ResponseClass::Success);
    assert_eq!(sent, expected);
}

/// Scenario: a GET endpoint whose path ends in a space.
///
/// Invariant: the payload is appended to the configured endpoint and the whole
/// target is then parsed, so the space is encoded as part of the path. ureq 2
/// parsed the same concatenated string, so a trimmed path would have sent a
/// different target.
#[rstest]
fn a_get_endpoint_is_parsed_after_the_payload_is_appended() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral listener");
    let (addr, requests) = spawn_mock_server(listener, 200).expect("spawn mock server");
    let config = HTTPHandlerConfig {
        url: format!("http://{addr}/log "),
        method: HTTPMethod::GET,
        ..Default::default()
    };

    execute(&config, &test_agent(), "msg=trailing").expect("the endpoint is sendable");

    let request = requests
        .recv_timeout(Duration::from_secs(5))
        .expect("the request reaches the listener");
    assert_eq!(request.path, "/log%20?msg=trailing");
}
