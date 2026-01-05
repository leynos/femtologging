//! Integration tests for the HTTP handler.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use rstest::{fixture, rstest};

use crate::handler::FemtoHandlerTrait;
use crate::handlers::{HTTPHandlerBuilder, HandlerBuilderTrait};
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;

use super::FemtoHTTPHandler;
use super::config::{HTTPHandlerConfig, HTTPMethod, SerializationFormat};
use super::worker::ResponseClass;

/// Spawn a mock HTTP server that captures the first request.
fn spawn_mock_server(
    listener: TcpListener,
    response_status: u16,
) -> (SocketAddr, mpsc::Receiver<CapturedRequest>) {
    spawn_retry_server(listener, vec![response_status])
}

fn status_text(code: u16) -> &'static str {
    match code {
        200 => "OK",
        201 => "Created",
        400 => "Bad Request",
        401 => "Unauthorized",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Unknown",
    }
}

#[derive(Debug)]
struct CapturedRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

/// Parses a single header line into a key-value pair.
fn parse_header_line(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    line.split_once(':')
        .map(|(key, value)| (key.trim().to_lowercase(), value.trim().to_string()))
}

/// Reads all headers from the request and returns them with the content length.
fn read_headers(reader: &mut BufReader<TcpStream>) -> (Vec<(String, String)>, usize) {
    let mut headers = Vec::new();
    let mut content_length = 0usize;

    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("read header");
        if line.trim().is_empty() {
            break;
        }
        let Some((key, value)) = parse_header_line(&line) else {
            continue;
        };
        if key == "content-length" {
            content_length = value.parse().unwrap_or(0);
        }
        headers.push((key, value));
    }

    (headers, content_length)
}

/// Reads the request body given the content length.
fn read_body(reader: &mut BufReader<TcpStream>, content_length: usize) -> String {
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).expect("read body");
    }
    String::from_utf8_lossy(&body).to_string()
}

fn read_http_request(stream: &mut TcpStream) -> CapturedRequest {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));

    // Read request line
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .expect("read request line");
    let parts: Vec<&str> = request_line.trim().split(' ').collect();
    let method = parts.first().unwrap_or(&"").to_string();
    let path = parts.get(1).unwrap_or(&"").to_string();

    let (headers, content_length) = read_headers(&mut reader);
    let body = read_body(&mut reader, content_length);

    CapturedRequest {
        method,
        path,
        headers,
        body,
    }
}

#[fixture]
fn tcp_listener() -> TcpListener {
    TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral listener")
}

fn build_http_handler(addr: SocketAddr) -> FemtoHTTPHandler {
    let url = format!("http://{}/log", addr);
    let config = HTTPHandlerConfig {
        url,
        method: HTTPMethod::POST,
        connect_timeout: Duration::from_secs(5),
        write_timeout: Duration::from_secs(5),
        ..Default::default()
    };
    FemtoHTTPHandler::with_config(config)
}

fn send_info_record(handler: &FemtoHTTPHandler, message: &str) {
    let record = FemtoLogRecord::new("test", FemtoLevel::Info, message);
    let _ = handler.handle(record);
}

#[rstest]
fn sends_records_over_http(tcp_listener: TcpListener) {
    let (addr, rx) = spawn_mock_server(tcp_listener, 200);
    let handler = build_http_handler(addr);
    send_info_record(&handler, "test message");

    let captured = rx.recv_timeout(Duration::from_secs(5)).expect("request");
    assert_eq!(captured.method, "POST");
    assert_eq!(captured.path, "/log");
    assert!(captured.body.contains("msg=test+message"));
    assert!(captured.body.contains("levelname=INFO"));

    drop(handler);
}

#[rstest]
fn sends_json_format(tcp_listener: TcpListener) {
    let (addr, rx) = spawn_mock_server(tcp_listener, 200);
    let url = format!("http://{}/log", addr);
    let config = HTTPHandlerConfig {
        url,
        format: SerializationFormat::Json,
        connect_timeout: Duration::from_secs(5),
        write_timeout: Duration::from_secs(5),
        ..Default::default()
    };
    let handler = FemtoHTTPHandler::with_config(config);
    send_info_record(&handler, "json test");

    let captured = rx.recv_timeout(Duration::from_secs(5)).expect("request");
    let content_type = captured
        .headers
        .iter()
        .find(|(k, _)| k == "content-type")
        .map(|(_, v)| v.as_str())
        .unwrap_or("");
    assert_eq!(content_type, "application/json");
    assert!(captured.body.contains("\"msg\":\"json test\""));

    drop(handler);
}

/// Helper function for testing authentication headers.
///
/// # Parameters
/// - `listener`: The TCP listener to use for the mock server
/// - `configure_auth`: Closure to configure authentication on the builder
/// - `verify_header`: Closure to verify the authorization header value
/// - `message`: The test message to send
fn test_auth_header<F, V>(listener: TcpListener, configure_auth: F, verify_header: V, message: &str)
where
    F: FnOnce(HTTPHandlerBuilder) -> HTTPHandlerBuilder,
    V: FnOnce(&str),
{
    let (addr, rx) = spawn_mock_server(listener, 200);
    let url = format!("http://{}/log", addr);
    let builder = HTTPHandlerBuilder::new().with_url(url);
    let handler = configure_auth(builder).build_inner().expect("build");
    send_info_record(&handler, message);

    let captured = rx.recv_timeout(Duration::from_secs(5)).expect("request");
    let auth = captured
        .headers
        .iter()
        .find(|(k, _)| k == "authorization")
        .map(|(_, v)| v.as_str())
        .unwrap_or("");
    verify_header(auth);

    drop(handler);
}

#[rstest]
fn sends_basic_auth_header(tcp_listener: TcpListener) {
    test_auth_header(
        tcp_listener,
        |builder| builder.with_basic_auth("user", "pass"),
        |auth| {
            assert!(auth.starts_with("Basic "));
            // "user:pass" base64 encoded is "dXNlcjpwYXNz"
            assert!(auth.contains("dXNlcjpwYXNz"));
        },
        "auth test",
    );
}

#[rstest]
fn sends_bearer_token(tcp_listener: TcpListener) {
    test_auth_header(
        tcp_listener,
        |builder| builder.with_bearer_token("my-secret-token"),
        |auth| {
            assert_eq!(auth, "Bearer my-secret-token");
        },
        "bearer test",
    );
}

#[rstest]
fn handler_closes_gracefully(tcp_listener: TcpListener) {
    let (addr, _rx) = spawn_mock_server(tcp_listener, 200);
    let mut handler = build_http_handler(addr);
    send_info_record(&handler, "close test");
    handler.close();
    // Should not panic or hang
}

/// Spawn a mock HTTP server that returns different status codes on successive requests.
///
/// The server handles requests sequentially, returning the status codes from the
/// provided slice in order. Once all statuses are exhausted, the server stops.
fn spawn_retry_server(
    listener: TcpListener,
    statuses: Vec<u16>,
) -> (SocketAddr, mpsc::Receiver<CapturedRequest>) {
    let addr = listener.local_addr().expect("listener has address");
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        for status in statuses {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            let captured = read_http_request(&mut stream);
            let response = format!(
                "HTTP/1.1 {} {}\r\nContent-Length: 0\r\n\r\n",
                status,
                status_text(status)
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = tx.send(captured);
        }
    });

    (addr, rx)
}

/// Helper function for testing retry behaviour.
///
/// # Parameters
/// - `listener`: The TCP listener to use for the mock server
/// - `statuses`: The sequence of HTTP status codes to return
/// - `message`: The test message to send
/// - `verify`: Closure to verify the captured requests
fn test_retry_behaviour<F>(listener: TcpListener, statuses: Vec<u16>, message: &str, verify: F)
where
    F: FnOnce(mpsc::Receiver<CapturedRequest>),
{
    use crate::socket_handler::BackoffPolicy;

    let (addr, rx) = spawn_retry_server(listener, statuses);
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
    send_info_record(&handler, message);

    verify(rx);

    drop(handler);
}

/// Verifies that the expected number of requests are received, each containing the expected message fragment.
fn verify_requests_with_message(
    rx: mpsc::Receiver<CapturedRequest>,
    count: usize,
    expected_msg_fragment: &str,
) {
    for _ in 0..count {
        let request = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("expected request");
        assert!(request.body.contains(expected_msg_fragment));
    }
}

#[rstest]
fn retries_on_503_then_succeeds(tcp_listener: TcpListener) {
    test_retry_behaviour(tcp_listener, vec![503, 200], "retry test", |rx| {
        verify_requests_with_message(rx, 2, "msg=retry+test");
    });
}

#[rstest]
fn retries_on_429_then_succeeds(tcp_listener: TcpListener) {
    test_retry_behaviour(tcp_listener, vec![429, 200], "rate limit test", |rx| {
        verify_requests_with_message(rx, 2, "msg=rate+limit+test");
    });
}

#[rstest]
fn does_not_retry_on_400(tcp_listener: TcpListener) {
    test_retry_behaviour(tcp_listener, vec![400], "permanent error test", |rx| {
        // Should receive exactly one request
        let first = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("first request");
        assert!(first.body.contains("msg=permanent+error+test"));

        // No second request should come (give it a short timeout to confirm)
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    });
}

// Response classification tests (unit tests for the worker module)
mod response_classification {
    use super::*;

    use crate::http_handler::worker::classify_status;

    #[rstest]
    #[case(200, ResponseClass::Success)]
    #[case(201, ResponseClass::Success)]
    #[case(204, ResponseClass::Success)]
    #[case(400, ResponseClass::Permanent)]
    #[case(401, ResponseClass::Permanent)]
    #[case(403, ResponseClass::Permanent)]
    #[case(404, ResponseClass::Permanent)]
    #[case(429, ResponseClass::Retryable)]
    #[case(500, ResponseClass::Retryable)]
    #[case(502, ResponseClass::Retryable)]
    #[case(503, ResponseClass::Retryable)]
    fn status_classification(#[case] status: u16, #[case] expected: ResponseClass) {
        assert_eq!(classify_status(status), expected);
    }
}
