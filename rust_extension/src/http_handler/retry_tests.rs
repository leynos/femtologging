//! Contracts for the HTTP worker's retry policy.
//!
//! These are about what is retried and what is not, in ordinary operation.
//! What a *shutdown* does to the same loop is a separate contract and lives in
//! `drain_tests.rs`: a retry policy and a deadline are two different bounds on
//! the same code, and a test that mixed them would not say which one it had
//! exercised.

use std::io;
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

use rstest::rstest;

use super::{
    CapturedRequest, FemtoHTTPHandler, send_info_record, spawn_retry_server, tcp_listener,
};
use crate::http_handler::config::{HTTPHandlerConfig, HTTPMethod};

/// Helper function for testing retry behaviour.
///
/// # Parameters
/// - `listener`: The TCP listener to use for the mock server
/// - `statuses`: The sequence of HTTP status codes to return
/// - `message`: The test message to send
/// - `verify`: Closure to verify the captured requests
fn test_retry_behaviour<F>(
    listener: TcpListener,
    statuses: Vec<u16>,
    message: &str,
    verify: F,
) -> io::Result<()>
where
    F: FnOnce(mpsc::Receiver<CapturedRequest>),
{
    use crate::socket_handler::BackoffPolicy;

    let (addr, rx) = spawn_retry_server(listener, statuses)?;
    let url = format!("http://{}/log", addr);
    let config = HTTPHandlerConfig {
        url,
        method: HTTPMethod::POST,
        connect_timeout: Duration::from_secs(5),
        write_timeout: Duration::from_secs(5),
        backoff: BackoffPolicy {
            base: Duration::from_millis(10),
            cap: Duration::from_millis(50),
            reset_after: Duration::from_secs(1),
            deadline: Duration::from_secs(5),
        },
        ..Default::default()
    };
    let handler = FemtoHTTPHandler::with_config(config);
    send_info_record(&handler, message).map_err(io::Error::other)?;

    verify(rx);

    drop(handler);
    Ok(())
}

/// Verifies that the expected number of requests are received, each containing the expected message fragment.
fn verify_requests_with_message(
    rx: mpsc::Receiver<CapturedRequest>,
    count: usize,
    expected_msg_fragment: &str,
) -> Result<(), mpsc::RecvTimeoutError> {
    for _ in 0..count {
        let request = rx.recv_timeout(Duration::from_secs(5))?;
        assert!(request.body.contains(expected_msg_fragment));
    }
    Ok(())
}

#[rstest]
fn retries_on_503_then_succeeds(tcp_listener: io::Result<TcpListener>) {
    let tcp_listener = tcp_listener.expect("bind ephemeral listener");
    test_retry_behaviour(tcp_listener, vec![503, 200], "retry test", |rx| {
        verify_requests_with_message(rx, 2, "msg=retry+test").expect("expected request");
    })
    .expect("retry test must complete");
}

#[rstest]
fn retries_on_429_then_succeeds(tcp_listener: io::Result<TcpListener>) {
    let tcp_listener = tcp_listener.expect("bind ephemeral listener");
    test_retry_behaviour(tcp_listener, vec![429, 200], "rate limit test", |rx| {
        verify_requests_with_message(rx, 2, "msg=rate+limit+test").expect("expected request");
    })
    .expect("retry test must complete");
}

#[rstest]
fn does_not_retry_on_400(tcp_listener: io::Result<TcpListener>) {
    let tcp_listener = tcp_listener.expect("bind ephemeral listener");
    test_retry_behaviour(tcp_listener, vec![400], "permanent error test", |rx| {
        // Should receive exactly one request
        let first = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("first request");
        assert!(first.body.contains("msg=permanent+error+test"));

        // No second request should come (give it a short timeout to confirm)
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    })
    .expect("retry test must complete");
}
