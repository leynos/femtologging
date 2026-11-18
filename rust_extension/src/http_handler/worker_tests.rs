//! Unit tests for the HTTP worker's own helpers.
//!
//! Response classification has its own file, `response_classification_tests.rs`,
//! and is not repeated here: what is left in the worker is the authentication
//! encoding that no other module owns, and the transport-failure
//! classification that keeps credentials out of its diagnostics.

use super::*;
use std::{
    collections::HashMap,
    io::{self, Read},
    net::{SocketAddr, TcpListener},
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

use crate::{http_handler::AuthConfig, level::FemtoLevel, shutdown_signal::ShutdownSignal};

#[test]
fn base64_encode_basic() {
    assert_eq!(base64_encode(b"user:pass"), "dXNlcjpwYXNz");
    assert_eq!(base64_encode(b"a"), "YQ==");
    assert_eq!(base64_encode(b"ab"), "YWI=");
    assert_eq!(base64_encode(b"abc"), "YWJj");
}

#[test]
#[serial_test::serial]
fn queued_shutdown_ack_waits_for_records_after_it() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind HTTP test listener");
    let address = listener.local_addr().expect("listener has address");
    let (request_tx, request_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept HTTP request");
        let mut first_byte = [0_u8; 1];
        stream
            .read_exact(&mut first_byte)
            .expect("read request start");
        request_tx.send(()).expect("notify that request started");
        release_rx.recv().expect("wait for test release");
    });

    let signal = Arc::new(ShutdownSignal::new());
    let config = HTTPHandlerConfig {
        url: format!("http://{address}/log"),
        connect_timeout: Duration::from_secs(1),
        write_timeout: Duration::from_secs(2),
        ..Default::default()
    };
    let mut worker = Worker::new(config, Arc::clone(&signal));
    let (command_tx, command_rx) = bounded(2);
    let (shutdown_tx, shutdown_rx) = bounded(1);
    command_tx
        .send(HTTPCommand::Shutdown(shutdown_tx))
        .expect("queue shutdown");
    command_tx
        .send(HTTPCommand::Record(FemtoLogRecord::new(
            "test",
            FemtoLevel::Info,
            "after shutdown",
        )))
        .expect("queue record behind shutdown");
    signal.begin_shutdown(Instant::now() + Duration::from_secs(2));

    let drain = thread::spawn(move || worker.drain_pending(&command_rx, None));
    request_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("record after shutdown should reach the HTTP worker");
    let acknowledgement_was_pending = matches!(
        shutdown_rx.try_recv(),
        Err(crossbeam_channel::TryRecvError::Empty)
    );

    release_tx
        .send(())
        .expect("release the stalled HTTP request");
    if acknowledgement_was_pending {
        shutdown_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("shutdown acknowledgement should follow the record");
    }
    drain.join().expect("worker drain should return");
    server.join().expect("HTTP server should return");
    assert!(
        acknowledgement_was_pending,
        "shutdown acknowledgement must wait until later queued records finish"
    );
}

#[test]
#[serial_test::serial]
fn aborted_request_counts_the_current_record_as_a_shutdown_drop() {
    crate::handlers::file::test_support::install_test_logger();
    let signal = Arc::new(ShutdownSignal::new());
    signal.abort();
    let mut worker = Worker::new(HTTPHandlerConfig::default(), signal);

    worker.send_request("payload");

    let messages = crate::handlers::file::test_support::take_logged_messages();
    assert!(messages.iter().any(|entry| {
        entry
            .message
            .contains("FemtoHTTPHandler dropped 1 records during shutdown")
    }));
}

#[test]
#[serial_test::serial]
fn shutdown_drop_is_logged_inside_the_warning_interval() {
    crate::handlers::file::test_support::install_test_logger();
    let worker = Worker::new(
        HTTPHandlerConfig::default(),
        Arc::new(ShutdownSignal::new()),
    );
    worker.warner.record_drop();
    worker.warner.warn_if_due(|_| {});

    worker.warn_shutdown_drops();

    let messages = crate::handlers::file::test_support::take_logged_messages();
    assert!(messages.iter().any(|entry| {
        entry
            .message
            .contains("FemtoHTTPHandler dropped 1 records during shutdown")
    }));
}

/// A password embedded in a synthetic URL's userinfo.
const SYNTHETIC_PASSWORD: &str = "SYNTHETIC_PASSWORD";
/// A query value embedded in the same synthetic URL.
const SYNTHETIC_QUERY_SECRET: &str = "SYNTHETIC_QUERY_SECRET";
/// A bearer token configured on the synthetic handler.
const SYNTHETIC_TOKEN_SECRET: &str = "SYNTHETIC_TOKEN_SECRET";
/// A secret inside a custom header value that `ureq` rejects.
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

/// Every classification the worker may report for a transport failure.
///
/// The list mirrors the arms of `transport_failure_classification`; a new
/// `ureq::ErrorKind` arm must extend it so the redaction tests keep their fixed
/// vocabulary.
const TRANSPORT_FAILURE_CLASSIFICATIONS: [&str; 13] = [
    "transport error (invalid URL)",
    "transport error (unknown scheme)",
    "transport error (DNS lookup failed)",
    "transport error (insecure request requires HTTPS)",
    "transport error (connection failed)",
    "transport error (too many redirects)",
    "transport error (invalid response status line)",
    "transport error (invalid header)",
    "transport error (I/O error)",
    "transport error (malformed proxy URL)",
    "transport error (proxy connection failed)",
    "transport error (proxy credentials rejected)",
    "transport error (unexpected HTTP status)",
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

/// Scenario: a configured URL carries userinfo and a query secret, plus a
/// bearer token, and the endpoint accepts the connection then closes without a
/// response.
///
/// Invariant: the failure is reported as an application-owned classification,
/// so neither the URL userinfo, its query value, nor the bearer token can reach
/// the warning the worker logs.
#[test]
fn transport_failure_does_not_expose_url_or_token_credentials() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback listener");
    let (address, server) = spawn_closing_server(listener);
    let config = HTTPHandlerConfig {
        url: format!(
            "http://synthetic-user:{SYNTHETIC_PASSWORD}@{address}/ingest\
             ?api_key={SYNTHETIC_QUERY_SECRET}"
        ),
        auth: AuthConfig::Bearer {
            token: SYNTHETIC_TOKEN_SECRET.to_owned(),
        },
        connect_timeout: REQUEST_ALLOWANCE,
        write_timeout: REQUEST_ALLOWANCE,
        ..Default::default()
    };
    let worker = Worker::new(config, Arc::new(ShutdownSignal::new()));

    let classification = worker
        .execute_request(SYNTHETIC_PAYLOAD_SECRET, &worker.agent)
        .expect_err("a closed connection without a response is a transport failure");

    server.join().expect("loopback server should return");
    assert_safe_classification(classification);
}

/// Scenario: a custom header value contains a CRLF and a synthetic secret, so
/// `ureq` rejects the header before connecting.
///
/// Invariant: the rejection is reported by classification only. The raw header
/// line, which repeats the secret, never appears in the returned diagnostic.
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
        .execute_request(SYNTHETIC_PAYLOAD_SECRET, &worker.agent)
        .expect_err("a header value containing CRLF is rejected before sending");

    drop(listener);
    assert_safe_classification(classification);
}
