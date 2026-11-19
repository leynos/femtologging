//! Contracts for the shape of one HTTP attempt.
//!
//! Each test drives the production transport against a listener on loopback, so
//! the assertions are about bytes on a socket rather than about the code that
//! produced them. They cover the parts of the ureq 2 to 3 migration that a
//! compile error cannot: which header value wins, where a GET payload goes, how
//! long an attempt may last, whether a response body is read, how a server
//! status is reported, where the redirect boundary sits, and what a redirect
//! does with the credentials.
//!
//! Nothing here leaves the host, changes the system trust store, or reads the
//! environment.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use rstest::rstest;

use super::{CapturedRequest, read_http_request, spawn_mock_server, tcp_listener};
use crate::http_handler::config::{AuthConfig, HTTPHandlerConfig, HTTPMethod, SerializationFormat};
use crate::http_handler::response::ResponseClass;
use crate::http_handler::transport::{build_agent, execute};

/// An agent with timeouts long enough that only the fixture ends an attempt.
fn test_agent() -> ureq::Agent {
    build_agent(Duration::from_secs(5), Duration::from_secs(5))
}

/// Every value sent for `name`, in wire order.
fn header_values<'a>(request: &'a CapturedRequest, name: &str) -> Vec<&'a str> {
    request
        .headers
        .iter()
        .filter(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
        .collect()
}

/// Spawn a server that runs `respond`, then holds the connection open.
///
/// The stream is held until `release` is signalled, so the client waits inside
/// its request rather than failing on a closed connection. The returned
/// receiver yields once `respond` has run. A `respond` error ends the thread
/// without the signal, so a broken fixture fails the test rather than being
/// mistaken for a client that stopped reading.
fn spawn_held_server<F>(
    listener: TcpListener,
    respond: F,
) -> io::Result<(SocketAddr, mpsc::Receiver<()>, mpsc::Sender<()>)>
where
    F: FnOnce(&mut TcpStream) -> io::Result<()> + Send + 'static,
{
    let addr = listener.local_addr()?;
    let (read_tx, read_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        if respond(&mut stream).is_err() {
            return;
        }
        let _ = read_tx.send(());
        let _ = release_rx.recv_timeout(Duration::from_secs(10));
    });
    Ok((addr, read_rx, release_tx))
}

/// Spawn a server that reads one request and never answers it.
fn spawn_silent_server(
    listener: TcpListener,
) -> io::Result<(SocketAddr, mpsc::Receiver<()>, mpsc::Sender<()>)> {
    spawn_held_server(listener, |stream| {
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut sink = [0_u8; 1024];
        stream.read(&mut sink).map(|_| ())
    })
}

/// Spawn a server that answers with a huge declared body and then stalls.
///
/// Only the status line and headers are written. A client that reads the body
/// waits for bytes that never arrive; one that stops at the verdict does not.
fn spawn_stalled_body_server(
    listener: TcpListener,
) -> io::Result<(SocketAddr, mpsc::Receiver<()>, mpsc::Sender<()>)> {
    spawn_held_server(listener, |stream| {
        read_http_request(stream)?;
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1048576\r\nConnection: close\r\n\r\n")
    })
}

/// Configuration that posts to `url` with the given auth and headers.
fn post_config(url: String, auth: AuthConfig, headers: &[(&str, &str)]) -> HTTPHandlerConfig {
    HTTPHandlerConfig {
        url,
        method: HTTPMethod::POST,
        auth,
        headers: headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect::<HashMap<_, _>>(),
        format: SerializationFormat::UrlEncoded,
        ..Default::default()
    }
}

/// Scenario: a custom `Authorization` header competes with the configured auth,
/// and a custom `Content-Type` competes with the format's content type.
///
/// Invariant: each header appears exactly once, with the value the ureq 2 code
/// would have sent. Configured auth is applied first, so the custom header
/// replaces it; the format's content type is applied last, so it replaces the
/// custom header. ureq 3 appends on every `header` call, so a naive port would
/// have sent two `Authorization` values - which a server is entitled to reject,
/// or to read as the wrong one.
#[rstest]
fn the_resolved_header_map_keeps_one_value_per_header(tcp_listener: io::Result<TcpListener>) {
    let tcp_listener = tcp_listener.expect("bind ephemeral listener");
    let (addr, requests) = spawn_mock_server(tcp_listener, 200).expect("spawn mock server");
    let config = post_config(
        format!("http://{addr}/log"),
        AuthConfig::Bearer {
            token: "configured-token".to_owned(),
        },
        &[
            ("authorization", "Bearer custom-token"),
            ("content-type", "text/plain"),
        ],
    );

    let class = execute(&config, &test_agent(), "msg=shape").expect("request should succeed");

    assert_eq!(class, ResponseClass::Success);
    let request = requests
        .recv_timeout(Duration::from_secs(5))
        .expect("request");
    assert_eq!(
        header_values(&request, "authorization"),
        ["Bearer custom-token"]
    );
    assert_eq!(
        header_values(&request, "content-type"),
        ["application/x-www-form-urlencoded"]
    );
}

/// Scenario: a GET request is sent to a URL with and without existing query
/// data.
///
/// Invariant: the payload is appended to the query string, joined with `&` when
/// a query already exists and with `?` when it does not.
#[rstest]
#[case("http://{addr}/log", "/log?msg=query+value")]
#[case("http://{addr}/log?existing=1", "/log?existing=1&msg=query+value")]
fn a_get_request_appends_its_payload_to_the_query(
    tcp_listener: io::Result<TcpListener>,
    #[case] url_template: &str,
    #[case] expected_path: &str,
) {
    let tcp_listener = tcp_listener.expect("bind ephemeral listener");
    let (addr, requests) = spawn_mock_server(tcp_listener, 200).expect("spawn mock server");
    let config = HTTPHandlerConfig {
        url: url_template.replace("{addr}", &addr.to_string()),
        method: HTTPMethod::GET,
        ..Default::default()
    };

    execute(&config, &test_agent(), "msg=query+value").expect("request should succeed");

    let request = requests
        .recv_timeout(Duration::from_secs(5))
        .expect("request");
    assert_eq!(request.method, "GET");
    assert_eq!(request.path, expected_path);
}

/// Scenario: the configured write timeout is short and the server never
/// answers.
///
/// Invariant: the attempt ends as a transport error near that timeout rather
/// than at the default. This is what pins `write_timeout` to ureq 3's
/// `timeout_global`; an agent left on its defaults would wait 30 seconds.
#[rstest]
fn the_write_timeout_bounds_an_unanswered_request(tcp_listener: io::Result<TcpListener>) {
    let tcp_listener = tcp_listener.expect("bind ephemeral listener");
    let (addr, read, release) = spawn_silent_server(tcp_listener).expect("spawn silent server");
    let write_timeout = Duration::from_millis(150);
    let agent = build_agent(Duration::from_secs(5), write_timeout);
    let config = post_config(format!("http://{addr}/log"), AuthConfig::None, &[]);

    let started = Instant::now();
    let result = execute(&config, &agent, "msg=timeout");
    let elapsed = started.elapsed();
    let _ = release.send(());

    assert!(
        result.is_err(),
        "a server that never answers must be a transport error: {result:?}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "the attempt must obey the {write_timeout:?} write timeout, not ureq's default: {elapsed:?}"
    );
    read.recv_timeout(Duration::from_secs(5))
        .expect("the request should reach the silent server");
}

/// Scenario: the server declares a body far larger than it ever sends.
///
/// Invariant: the attempt ends at the status line. The worker never reads a
/// response body, so a body that stalls must not hold the attempt open until
/// the request timeout - and must not turn a delivered record into a retry.
#[rstest]
fn a_response_body_that_never_arrives_does_not_hold_the_attempt(
    tcp_listener: io::Result<TcpListener>,
) {
    let tcp_listener = tcp_listener.expect("bind ephemeral listener");
    let (addr, read, release) =
        spawn_stalled_body_server(tcp_listener).expect("spawn stalled-body server");
    let agent = test_agent();
    let config = post_config(format!("http://{addr}/log"), AuthConfig::None, &[]);

    let started = Instant::now();
    let class = execute(&config, &agent, "msg=body");
    let elapsed = started.elapsed();
    let _ = release.send(());

    assert_eq!(
        class.expect("the status line is enough for a verdict"),
        ResponseClass::Success
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "reading the declared body would have waited out the request timeout: {elapsed:?}"
    );
    read.recv_timeout(Duration::from_secs(5))
        .expect("the request should reach the stalled-body server");
}

/// Scenario: the server answers 503.
///
/// Invariant: the status is a verdict the retry policy sees, not a transport
/// error. `http_status_as_error(false)` is what keeps the worker's own
/// classification in charge of retries.
#[rstest]
fn a_server_error_status_is_a_retryable_verdict(tcp_listener: io::Result<TcpListener>) {
    let tcp_listener = tcp_listener.expect("bind ephemeral listener");
    let (addr, _requests) = spawn_mock_server(tcp_listener, 503).expect("spawn mock server");
    let config = post_config(format!("http://{addr}/log"), AuthConfig::None, &[]);

    let class =
        execute(&config, &test_agent(), "msg=retry").expect("a 503 is not a transport error");

    assert_eq!(class, ResponseClass::Retryable);
}

/// Scenario: the connect timeout is far longer than the global timeout and the
/// peer accepts the TCP connection but never answers the TLS handshake.
///
/// Invariant: the attempt ends near the global timeout. ureq 3 checks its
/// global timeout in every phase, connection establishment included, so the
/// handler's `write_timeout` - the global timeout - is what bounds a connect,
/// not `connect_timeout`. ureq 2 exempted its connect phase from the overall
/// deadline; the tighter bound is the one this migration accepts, because a
/// connection that outlives the attempt budget could never deliver the record
/// and both versions report a transport error.
#[rstest]
fn the_global_timeout_bounds_connection_establishment(tcp_listener: io::Result<TcpListener>) {
    let tcp_listener = tcp_listener.expect("bind ephemeral listener");
    let (addr, _read, release) = spawn_held_server(tcp_listener, |_stream: &mut TcpStream| Ok(()))
        .expect("spawn stalled-handshake server");
    let connect_timeout = Duration::from_secs(10);
    let global_timeout = Duration::from_millis(200);
    let agent = build_agent(connect_timeout, global_timeout);
    let config = HTTPHandlerConfig {
        url: format!("https://127.0.0.1:{}/log", addr.port()),
        ..Default::default()
    };

    let started = Instant::now();
    let result = execute(&config, &agent, "msg=handshake");
    let elapsed = started.elapsed();
    let _ = release.send(());

    assert!(
        result.is_err(),
        "a handshake that never completes must be a transport error: {result:?}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "the {global_timeout:?} global timeout must bound a connect, not the {connect_timeout:?} connect timeout: {elapsed:?}"
    );
}

// Redirect contracts - the follow limit and credential stripping - with their
// loopback fixtures.
#[cfg(test)]
#[path = "request_shape_tests/redirects.rs"]
mod redirects;

// Redirect responses that are classified rather than followed, which ureq 3
// would otherwise follow or report as transport errors.
#[cfg(test)]
#[path = "request_shape_tests/redirect_responses.rs"]
mod redirect_responses;

// Endpoint encoding - the bytes a configured path produces on the wire.
#[cfg(test)]
#[path = "request_shape_tests/endpoint.rs"]
mod endpoint;
