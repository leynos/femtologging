//! Contracts for the shape of one HTTP attempt.
//!
//! Each test drives the production transport against a listener on loopback, so
//! the assertions are about bytes on a socket rather than about the code that
//! produced them. They cover the parts of the ureq 2 to 3 migration that a
//! compile error cannot: which header value wins, where a GET payload goes, how
//! long an attempt may last, whether a response body is read, how a server
//! status is reported, and what a redirect does with the credentials.
//!
//! Nothing here leaves the host, changes the system trust store, or reads the
//! environment.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use rstest::rstest;
use ureq::tls::RootCerts;

use super::{CapturedRequest, read_http_request, spawn_mock_server, tcp_listener};
use crate::http_handler::config::{AuthConfig, HTTPHandlerConfig, HTTPMethod, SerializationFormat};
use crate::http_handler::response::ResponseClass;
use crate::http_handler::transport::{build_agent, execute};

/// An agent with timeouts long enough that only the fixture ends an attempt.
fn test_agent() -> ureq::Agent {
    build_agent(
        Duration::from_secs(5),
        Duration::from_secs(5),
        RootCerts::PlatformVerifier,
    )
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

/// Spawn a server that reads one request and never answers it.
///
/// The stream is held until `release` is signalled, so the client waits inside
/// its request rather than failing on a closed connection. The returned
/// receiver yields once the request has been read.
fn spawn_silent_server(
    listener: TcpListener,
) -> io::Result<(SocketAddr, mpsc::Receiver<()>, mpsc::Sender<()>)> {
    let addr = listener.local_addr()?;
    let (read_tx, read_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut sink = [0_u8; 1024];
        let _ = stream.read(&mut sink);
        let _ = read_tx.send(());
        let _ = release_rx.recv_timeout(Duration::from_secs(10));
    });
    Ok((addr, read_rx, release_tx))
}

/// Spawn a server that answers with a huge declared body and then stalls.
///
/// Only the status line and headers are written. A client that reads the body
/// waits for bytes that never arrive; one that stops at the verdict does not.
fn spawn_stalled_body_server(
    listener: TcpListener,
) -> io::Result<(SocketAddr, mpsc::Receiver<()>, mpsc::Sender<()>)> {
    let addr = listener.local_addr()?;
    let (read_tx, read_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let Ok(_request) = read_http_request(&mut stream) else {
            return;
        };
        let _ = stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1048576\r\nConnection: close\r\n\r\n");
        let _ = read_tx.send(());
        let _ = release_rx.recv_timeout(Duration::from_secs(10));
    });
    Ok((addr, read_rx, release_tx))
}

/// Spawn a server that answers its one request with a redirect to `location`.
fn spawn_redirect_server(
    listener: TcpListener,
    location: String,
) -> io::Result<(SocketAddr, mpsc::Receiver<CapturedRequest>)> {
    let addr = listener.local_addr()?;
    let (capture_tx, capture_rx) = mpsc::channel();
    thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let Ok(request) = read_http_request(&mut stream) else {
            return;
        };
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = capture_tx.send(request);
    });
    Ok((addr, capture_rx))
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
    let agent = build_agent(
        Duration::from_secs(5),
        write_timeout,
        RootCerts::PlatformVerifier,
    );
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

/// Scenario: the collector redirects to a second listener.
///
/// Invariant: the redirect is followed, but the credentials sent to the
/// original host are not replayed at the target. ureq 3's default for
/// `redirect_auth_headers` differs from ureq 2's, so this is pinned explicitly.
#[rstest]
fn a_redirect_does_not_forward_the_authorization_header(tcp_listener: io::Result<TcpListener>) {
    let redirect_listener = tcp_listener.expect("bind ephemeral listener");
    let target_listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind redirect target");
    let (target_addr, target_requests) =
        spawn_mock_server(target_listener, 200).expect("spawn redirect target");
    let (redirect_addr, redirect_requests) =
        spawn_redirect_server(redirect_listener, format!("http://{target_addr}/collected"))
            .expect("spawn redirect server");
    let config = post_config(
        format!("http://{redirect_addr}/log"),
        AuthConfig::Bearer {
            token: "redirect-secret".to_owned(),
        },
        &[],
    );

    let class = execute(&config, &test_agent(), "msg=redirect").expect("redirect is followed");

    assert_eq!(class, ResponseClass::Success);
    let first = redirect_requests
        .recv_timeout(Duration::from_secs(5))
        .expect("the first request should carry the configured auth");
    assert_eq!(
        header_values(&first, "authorization"),
        ["Bearer redirect-secret"]
    );
    let second = target_requests
        .recv_timeout(Duration::from_secs(5))
        .expect("the redirect target should be reached");
    assert_eq!(second.path, "/collected");
    assert!(
        header_values(&second, "authorization").is_empty(),
        "a redirect must not replay credentials: {:?}",
        header_values(&second, "authorization")
    );
}
