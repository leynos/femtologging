//! Credential-free transport classifications for the worker's retry warning.
//!
//! The worker reports a failed attempt through `log::warn!`. These tests use a
//! loopback endpoint that closes unanswered and synthetic credentials in the
//! URL, the bearer token, and a rejected header value, then assert that the
//! classification the worker returns is known, exact where the path is fixed,
//! and free of every sentinel.

use super::*;
use std::{
    collections::HashMap,
    io::{self, Read},
    net::{SocketAddr, TcpListener},
    thread,
    time::{Duration, Instant},
};

use crate::http_handler::{AuthConfig, HTTPMethod};

/// A password embedded in a synthetic URL's userinfo.
const SYNTHETIC_PASSWORD: &str = "SYNTHETIC_PASSWORD";
/// A query value embedded in the same synthetic URL.
const SYNTHETIC_QUERY_SECRET: &str = "SYNTHETIC_QUERY_SECRET";
/// A bearer token configured on the synthetic handler.
const SYNTHETIC_TOKEN_SECRET: &str = "SYNTHETIC_TOKEN_SECRET";
/// A secret inside a custom header value that cannot be encoded.
const SYNTHETIC_HEADER_SECRET: &str = "SYNTHETIC_HEADER_SECRET";
/// A record message that must not reach the transport diagnostic.
const SYNTHETIC_PAYLOAD_SECRET: &str = "SYNTHETIC_PAYLOAD_SECRET";

/// Every sentinel that must be absent from transport-failure diagnostics.
const SYNTHETIC_SECRETS: [&str; 5] = [
    SYNTHETIC_PASSWORD,
    SYNTHETIC_QUERY_SECRET,
    SYNTHETIC_TOKEN_SECRET,
    SYNTHETIC_HEADER_SECRET,
    SYNTHETIC_PAYLOAD_SECRET,
];

/// Every label the send path can hand the retry warning.
///
/// The transport labels mirror `transport/failure.rs`; `invalid header`
/// mirrors the send-time header encoding; `attempt ran out of time` mirrors the
/// exhausted budget; and the redirect labels mirror `redirect.rs`. A new arm
/// must extend this fixed vocabulary so the redaction tests keep asserting a
/// known label.
const TRANSPORT_FAILURE_CLASSIFICATIONS: [&str; 23] = [
    "transport error (unexpected HTTP status)",
    "transport error (invalid request)",
    "transport error (invalid URL)",
    "transport error (protocol error)",
    "transport error (I/O error)",
    "transport error (timed out)",
    "transport error (DNS lookup failed)",
    "transport error (redirect refused)",
    "transport error (malformed proxy URL)",
    "transport error (connection failed)",
    "transport error (request body too large)",
    "transport error (too many redirects)",
    "transport error (TLS failure)",
    "transport error (insecure request requires HTTPS)",
    "transport error (response header too large)",
    "transport error (proxy connection failed)",
    "transport error (TLS required)",
    "transport error (unrecognised failure)",
    "transport error (response body stalled)",
    "transport error (invalid header)",
    "transport error (attempt ran out of time)",
    "reached max redirects",
    "invalid redirect location",
];

/// How long a loopback server waits for the worker to connect.
const ACCEPT_ALLOWANCE: Duration = Duration::from_secs(5);
/// How long an accepted stream waits for the request bytes.
const READ_ALLOWANCE: Duration = Duration::from_secs(5);
/// How long the worker's own connect and request attempts may take.
const REQUEST_ALLOWANCE: Duration = Duration::from_secs(2);

/// Spawn a loopback server that accepts one connection and closes unanswered.
///
/// Both waits are bounded: the listener polls for the connection and the
/// accepted stream carries a read timeout. Dropping the stream without writing
/// a response is what makes `ureq` report a transport failure.
fn spawn_closing_server(listener: TcpListener) -> (SocketAddr, thread::JoinHandle<()>) {
    let address = listener.local_addr().expect("listener has address");
    let server = thread::spawn(move || {
        listener
            .set_nonblocking(true)
            .expect("listener can poll for a connection");
        let deadline = Instant::now() + ACCEPT_ALLOWANCE;
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "the worker should connect within {ACCEPT_ALLOWANCE:?}"
                    );
                    thread::sleep(Duration::from_millis(5));
                }
                Err(err) => panic!("accept the worker's connection: {err}"),
            }
        };
        stream
            .set_read_timeout(Some(READ_ALLOWANCE))
            .expect("accepted stream accepts a read timeout");
        let mut sink = [0_u8; 1024];
        let _ = stream.read(&mut sink);
    });
    (address, server)
}

/// The next attempt a worker would send, bounded so a stalled peer cannot hang
/// the test.
fn next_attempt(worker: &Worker) -> Attempt {
    Attempt {
        agent: worker.agent.clone(),
        budget: REQUEST_ALLOWANCE,
    }
}

/// A backoff policy that gives a record exactly one attempt.
///
/// A zero deadline makes `next_sleep` decline immediately, so the retry path
/// logs its warning and then the retry-deadline drop count without sleeping.
fn single_attempt_backoff() -> crate::socket_handler::BackoffPolicy {
    crate::socket_handler::BackoffPolicy {
        base: Duration::from_millis(1),
        cap: Duration::from_millis(1),
        reset_after: Duration::from_secs(60),
        deadline: Duration::ZERO,
    }
}

/// Scenario: the retry path is reached with a classification and a spent
/// backoff deadline, so it logs the warning and then the drop count.
///
/// Invariant: the warning text is exactly the classification the caller
/// passed. Nothing else is appended to it, so a label the worker was handed is
/// the whole diagnostic and no surrounding context can reintroduce what the
/// label omitted.
#[test]
#[serial_test::serial]
fn the_retry_warning_is_exactly_the_classification() {
    crate::handlers::file::test_support::install_test_logger();
    let worker = Worker::new(
        HTTPHandlerConfig {
            backoff: single_attempt_backoff(),
            ..Default::default()
        },
        Arc::new(ShutdownSignal::new()),
    );
    let mut worker = worker;

    let retried = worker.should_retry("transport error (invalid header)", Instant::now());

    assert!(!retried, "a spent backoff deadline must end the retries");
    let messages = crate::handlers::file::test_support::take_logged_messages();
    let warning = messages
        .iter()
        .find(|entry| entry.message.contains("request failed"))
        .expect("the retry path must log its warning");
    assert_eq!(
        warning.message,
        "FemtoHTTPHandler request failed: transport error (invalid header)"
    );
}

/// Assert a classification is known, non-empty, and free of every sentinel.
fn assert_safe_classification(classification: &'static str) {
    assert!(
        !classification.is_empty(),
        "a transport classification must not be empty"
    );
    assert!(
        TRANSPORT_FAILURE_CLASSIFICATIONS.contains(&classification),
        "unexpected transport classification: {classification:?}"
    );
    for secret in SYNTHETIC_SECRETS {
        assert!(
            !classification.contains(secret),
            "transport classification leaked {secret}: {classification:?}"
        );
    }
}

/// Scenario: a URL carries userinfo and a query secret, plus a bearer token,
/// and the endpoint accepts the connection then closes without a response. A
/// GET payload additionally becomes part of that request target.
///
/// Invariant: both methods report an application-owned classification, so
/// neither the URL userinfo, its query value, the bearer token, nor the payload
/// can reach the warning the worker logs.
#[rstest::rstest]
#[case::post(HTTPMethod::POST)]
#[case::get(HTTPMethod::GET)]
fn transport_failure_does_not_expose_url_or_token_credentials(#[case] method: HTTPMethod) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback listener");
    let (address, server) = spawn_closing_server(listener);
    let config = HTTPHandlerConfig {
        url: format!(
            "http://synthetic-user:{SYNTHETIC_PASSWORD}@{address}/ingest\
             ?api_key={SYNTHETIC_QUERY_SECRET}"
        ),
        method,
        auth: AuthConfig::Bearer {
            token: SYNTHETIC_TOKEN_SECRET.to_owned(),
        },
        connect_timeout: REQUEST_ALLOWANCE,
        write_timeout: REQUEST_ALLOWANCE,
        ..Default::default()
    };
    let worker = Worker::new(config, Arc::new(ShutdownSignal::new()));

    let classification = worker
        .execute_request(SYNTHETIC_PAYLOAD_SECRET, &next_attempt(&worker))
        .expect_err("a closed connection without a response is a transport failure");

    server.join().expect("loopback server should return");
    assert_eq!(classification, "transport error (I/O error)");
    assert_safe_classification(classification);
}

/// Scenario: a custom header value contains a CRLF and a synthetic secret, so
/// the send path rejects the header before connecting.
///
/// Invariant: the rejection is reported as the fixed `invalid header`
/// classification. The raw header line, which repeats the secret, never appears
/// in the returned diagnostic.
#[test]
fn transport_failure_does_not_expose_a_rejected_header_value() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback listener");
    let address = listener.local_addr().expect("listener has address");
    let headers = HashMap::from([(
        "X-Synthetic-Secret".to_owned(),
        format!("{SYNTHETIC_HEADER_SECRET}\r\nInjected: value"),
    )]);
    let config = HTTPHandlerConfig {
        url: format!("http://{address}/ingest"),
        headers,
        connect_timeout: REQUEST_ALLOWANCE,
        write_timeout: REQUEST_ALLOWANCE,
        ..Default::default()
    };
    let worker = Worker::new(config, Arc::new(ShutdownSignal::new()));

    let classification = worker
        .execute_request(SYNTHETIC_PAYLOAD_SECRET, &next_attempt(&worker))
        .expect_err("a header value containing CRLF is rejected before sending");

    drop(listener);
    assert_eq!(classification, "transport error (invalid header)");
    assert_safe_classification(classification);
}
