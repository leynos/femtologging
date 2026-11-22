//! Regressions that keep HTTP request credentials out of the `log` bridge's
//! diagnostic output.
//!
//! The HTTP worker reports a transient failure through `log::warn!`. With the
//! bridge installed, a warning from this crate's own targets is mirrored to
//! stderr rather than dispatched back into the adapter, which makes the
//! process's captured output the exact diagnostic a caller would see. These
//! tests drive the real handler against loopback endpoints carrying synthetic
//! credentials and assert that none of them survive into that output.

use std::{
    collections::HashMap,
    io::{self, Read},
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};

use crate::handler::FemtoHandlerTrait;
use crate::http_handler::{AuthConfig, FemtoHTTPHandler, HTTPHandlerConfig};
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;
use crate::socket_handler::BackoffPolicy;

use super::install_global_logger;

/// A password embedded in the synthetic URL's userinfo.
const SYNTHETIC_PASSWORD: &str = "SYNTHETIC_PASSWORD";
/// A query value embedded in the synthetic URL.
const SYNTHETIC_QUERY_SECRET: &str = "SYNTHETIC_QUERY_SECRET";
/// A bearer token configured on the synthetic handler.
const SYNTHETIC_TOKEN_SECRET: &str = "SYNTHETIC_TOKEN_SECRET";
/// A secret inside a custom header value that `ureq` rejects.
const SYNTHETIC_HEADER_SECRET: &str = "SYNTHETIC_HEADER_SECRET";
/// A record message carried by the enqueued log record.
const SYNTHETIC_PAYLOAD_SECRET: &str = "SYNTHETIC_PAYLOAD_SECRET";

/// Every sentinel that must be absent from the captured output.
const SYNTHETIC_SECRETS: [&str; 5] = [
    SYNTHETIC_PASSWORD,
    SYNTHETIC_QUERY_SECRET,
    SYNTHETIC_TOKEN_SECRET,
    SYNTHETIC_HEADER_SECRET,
    SYNTHETIC_PAYLOAD_SECRET,
];

/// How long the mock endpoint waits for the worker to connect.
const ACCEPT_ALLOWANCE: Duration = Duration::from_secs(5);
/// How long the mock endpoint waits for the request bytes.
const READ_ALLOWANCE: Duration = Duration::from_secs(5);
/// How long one connect attempt may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// How long flush and close may wait for the worker.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(5);

/// Spawn a loopback server that accepts one connection and closes unanswered.
///
/// The listener polls for the connection and the accepted stream carries a read
/// timeout, so neither wait outlives the test. Dropping the stream without
/// writing a response is the transport failure under test.
fn spawn_closing_server(listener: TcpListener) -> (String, thread::JoinHandle<()>) {
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
    (address.to_string(), server)
}

/// A backoff policy that gives each record exactly one attempt.
///
/// A zero deadline makes `next_sleep` decline immediately, so the worker logs
/// the transport failure and then the retry-deadline drop warning without
/// sleeping or retrying.
fn single_attempt_backoff() -> BackoffPolicy {
    BackoffPolicy {
        base: Duration::from_millis(1),
        cap: Duration::from_millis(1),
        reset_after: Duration::from_secs(60),
        deadline: Duration::ZERO,
    }
}

/// Build a handler aimed at `url` with the supplied request credentials.
fn build_redaction_handler(
    url: String,
    auth: AuthConfig,
    headers: HashMap<String, String>,
) -> FemtoHTTPHandler {
    FemtoHTTPHandler::with_config(HTTPHandlerConfig {
        url,
        auth,
        headers,
        connect_timeout: CONNECT_TIMEOUT,
        write_timeout: FLUSH_TIMEOUT,
        backoff: single_attempt_backoff(),
        ..Default::default()
    })
}

/// Queue one record and wait for the worker to acknowledge the flush.
fn send_and_flush(handler: &FemtoHTTPHandler, message: &str) {
    let record = FemtoLogRecord::new("redaction.child", FemtoLevel::Info, message);
    handler
        .handle(record)
        .expect("the worker should accept one record");
    assert!(
        handler.flush(),
        "the worker should acknowledge the flush after one failed attempt"
    );
}

/// Exercise both credential-bearing failure paths in a fresh process.
///
/// The child owns the global Rust logger, so the worker's warnings travel
/// through the bridge and are mirrored to stderr. The parent asserts that the
/// captured output reports the failures without any synthetic secret.
#[test]
#[ignore = "run in a fresh process because the global Rust logger is process-global"]
fn http_transport_redaction_child() {
    assert!(
        install_global_logger(),
        "the child should own the global Rust logger"
    );

    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback listener");
    let (address, server) = spawn_closing_server(listener);
    let url = format!(
        "http://synthetic-user:{SYNTHETIC_PASSWORD}@{address}/ingest\
         ?api_key={SYNTHETIC_QUERY_SECRET}"
    );
    let mut handler = build_redaction_handler(
        url,
        AuthConfig::Bearer {
            token: SYNTHETIC_TOKEN_SECRET.to_owned(),
        },
        HashMap::new(),
    );
    send_and_flush(&handler, SYNTHETIC_PAYLOAD_SECRET);
    handler.close();
    server.join().expect("loopback server should return");

    // A rejected header fails validation before any connection is attempted,
    // so this endpoint only has to be a valid loopback address.
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback listener");
    let address = listener.local_addr().expect("listener has address");
    let headers = HashMap::from([(
        "X-Synthetic-Secret".to_owned(),
        format!("{SYNTHETIC_HEADER_SECRET}\r\nInjected: value"),
    )]);
    let mut handler = build_redaction_handler(
        format!("http://{address}/ingest"),
        AuthConfig::None,
        headers,
    );
    send_and_flush(&handler, SYNTHETIC_PAYLOAD_SECRET);
    handler.close();
    drop(listener);
}

/// Scenario: the handler reports two credential-bearing transport failures in
/// an isolated child process.
///
/// Invariant: the captured output carries a classification for each failure and
/// the retry-deadline drop warning, and none of the synthetic password, query
/// value, bearer token, header value, or record payload appears in it.
#[test]
fn transport_failure_warnings_never_include_configured_credentials() {
    let child_filter = "http_transport_redaction_child";
    let output = crate::test_utils::isolated_child::run_isolated_test(child_filter)
        .expect("the isolated redaction test should start");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "isolated test {child_filter:?} failed:\nstdout:\n{stdout}\nstderr:\n{stderr}",
    );

    // The bridge mirrors a crate-target warning to stderr, so that is the
    // stream the caller's diagnostic output actually comes from. Requiring the
    // warnings there keeps this an assertion about the bridge rather than
    // about whichever stream happened to carry the text.
    let failure_lines = stderr
        .matches("FemtoHTTPHandler request failed: transport error (")
        .count();
    assert!(
        failure_lines >= 2,
        "each failing handler should mirror one transport classification to \
         stderr, saw {failure_lines}:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // The report names its cause since the drop accounting was unified, so the
    // assertion follows the current wording rather than the old sentence.
    assert!(
        stderr.contains("triggering cause: retry deadline exhaustion"),
        "the retry-deadline drop warning should still reach stderr:\n{stderr}"
    );

    let diagnostics = format!("{stdout}\n{stderr}");
    for secret in SYNTHETIC_SECRETS {
        assert!(
            !diagnostics.contains(secret),
            "diagnostic output leaked {secret}:\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }
}
