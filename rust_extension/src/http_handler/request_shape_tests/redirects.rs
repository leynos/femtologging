//! Redirect contracts: the follow limit, credential stripping, and the one
//! redirect ureq 3 cannot follow.
//!
//! The fixtures serve redirect chains on loopback. They live beside the request
//! shape tests because they share the same server helpers, and in their own
//! file so each stays readable.

use std::io::{self, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use rstest::rstest;

use super::{
    AuthConfig, CapturedRequest, ResponseClass, execute, header_values, post_config,
    read_http_request, spawn_mock_server, tcp_listener, test_agent,
};

/// Spawn a server that answers its one request with `status` and `location`.
fn spawn_redirect_server(
    listener: TcpListener,
    status: u16,
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
            "HTTP/1.1 {status} Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = capture_tx.send(request);
    });
    Ok((addr, capture_rx))
}

/// What one non-blocking accept attempt found.
enum AcceptAttempt {
    /// A connection arrived and is ready to read.
    Connected(TcpStream),
    /// No connection is pending yet, but the chain is still active.
    Pending,
    /// The fixture should stop: the chain went idle or the listener failed.
    Finished,
}

/// Attempt one accept and classify what the caller should do next.
fn accept_once(
    listener: &TcpListener,
    last_connection: Instant,
    idle_limit: Duration,
) -> AcceptAttempt {
    match listener.accept() {
        Ok((stream, _)) => {
            // A non-blocking listener's accepted socket is blocking on
            // Linux, but not necessarily everywhere.
            let _ = stream.set_nonblocking(false);
            AcceptAttempt::Connected(stream)
        }
        Err(err) if err.kind() != io::ErrorKind::WouldBlock => AcceptAttempt::Finished,
        Err(_) if last_connection.elapsed() > idle_limit => AcceptAttempt::Finished,
        Err(_) => AcceptAttempt::Pending,
    }
}

/// Wait for the next connection, giving up once the chain has been idle.
///
/// Returns `None` when no connection arrived before the idle limit or the
/// listener failed, so the caller can stop the fixture. A client that refuses a
/// redirect never opens another connection, which makes this wait the way the
/// fixture ends.
fn accept_before_idle(
    listener: &TcpListener,
    last_connection: Instant,
    idle_limit: Duration,
) -> Option<TcpStream> {
    loop {
        match accept_once(listener, last_connection, idle_limit) {
            AcceptAttempt::Connected(stream) => return Some(stream),
            AcceptAttempt::Finished => return None,
            AcceptAttempt::Pending => thread::sleep(Duration::from_millis(5)),
        }
    }
}

/// Spawn a server that answers a chain of `redirects` redirects on one address.
///
/// Each request is answered with a redirect to the next path until the chain
/// has used its redirects; the request after that is answered 200. A client
/// that refuses a redirect stops opening connections, so the accept loop gives
/// up after a short idle period rather than block forever on a connection that
/// is never coming - a hung fixture would hide the regression the tests are
/// looking for.
fn spawn_redirect_chain(
    listener: TcpListener,
    redirects: usize,
) -> io::Result<(SocketAddr, thread::JoinHandle<()>)> {
    let addr = listener.local_addr()?;
    listener.set_nonblocking(true)?;
    let handle = thread::spawn(move || {
        let idle_limit = Duration::from_secs(2);
        let mut last_connection = Instant::now();
        for hop in 0..=redirects {
            let Some(mut stream) = accept_before_idle(&listener, last_connection, idle_limit)
            else {
                return;
            };
            if read_http_request(&mut stream).is_err() {
                return;
            }
            let response = if hop < redirects {
                format!(
                    "HTTP/1.1 302 Found\r\nLocation: http://{addr}/hop{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    hop + 1
                )
            } else {
                "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()
            };
            if stream.write_all(response.as_bytes()).is_err() {
                return;
            }
            last_connection = Instant::now();
        }
    });
    Ok((addr, handle))
}

/// Run one attempt against a chain of `redirects` redirects.
fn execute_redirect_chain(
    listener: TcpListener,
    redirects: usize,
) -> Result<ResponseClass, String> {
    let (addr, server) = spawn_redirect_chain(listener, redirects).expect("spawn redirect chain");
    let config = post_config(format!("http://{addr}/hop0"), AuthConfig::None, &[]);

    let result = execute(&config, &test_agent(), "msg=redirect");
    server.join().expect("redirect chain server should finish");
    result
}

/// Scenario: the collector redirects to a second listener, and the request
/// carries both the configured auth and a custom credential header.
///
/// Invariant: the redirect is followed, but the `Authorization` header sent to
/// the original host is not replayed at the target. ureq 3's
/// `RedirectAuthHeaders` setting covers only `Authorization`, so a custom
/// credential header is forwarded; that is also what ureq 2 did, and the
/// documentation states the distinction rather than claiming every credential
/// is stripped.
#[rstest]
fn a_redirect_strips_authorization_and_forwards_custom_headers(
    tcp_listener: io::Result<TcpListener>,
) {
    let redirect_listener = tcp_listener.expect("bind ephemeral listener");
    let target_listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind redirect target");
    let (target_addr, target_requests) =
        spawn_mock_server(target_listener, 200).expect("spawn redirect target");
    let (redirect_addr, redirect_requests) = spawn_redirect_server(
        redirect_listener,
        302,
        format!("http://{target_addr}/collected"),
    )
    .expect("spawn redirect server");
    let config = post_config(
        format!("http://{redirect_addr}/log"),
        AuthConfig::Bearer {
            token: "redirect-secret".to_owned(),
        },
        &[("x-api-key", "custom-credential")],
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
    assert_eq!(header_values(&first, "x-api-key"), ["custom-credential"]);
    let second = target_requests
        .recv_timeout(Duration::from_secs(5))
        .expect("the redirect target should be reached");
    assert_eq!(second.path, "/collected");
    assert!(
        header_values(&second, "authorization").is_empty(),
        "a redirect must not replay the Authorization header: {:?}",
        header_values(&second, "authorization")
    );
    assert_eq!(
        header_values(&second, "x-api-key"),
        ["custom-credential"],
        "a custom header is outside RedirectAuthHeaders and is forwarded"
    );
}

/// Scenario: a collector answers a POST with 307 or 308.
///
/// Invariant: the verdict is permanent, exactly as it was under ureq 2, which
/// returned the redirect response instead of replaying the request body. ureq 3
/// reports the same refusal as `RedirectFailed`; left unhandled it would become
/// a transport error and the record would be retried until the backoff
/// deadline. The target is never contacted, which is why the response has to be
/// classified rather than followed.
#[rstest]
#[case(307)]
#[case(308)]
fn a_post_that_cannot_be_replayed_is_permanent(
    tcp_listener: io::Result<TcpListener>,
    #[case] status: u16,
) {
    let redirect_listener = tcp_listener.expect("bind ephemeral listener");
    let target_listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind redirect target");
    target_listener
        .set_nonblocking(true)
        .expect("the target listener must not block the assertion");
    let target_addr = target_listener.local_addr().expect("target address");
    let (redirect_addr, requests) = spawn_redirect_server(
        redirect_listener,
        status,
        format!("http://{target_addr}/moved"),
    )
    .expect("spawn redirect server");
    let config = post_config(format!("http://{redirect_addr}/log"), AuthConfig::None, &[]);

    let class = execute(&config, &test_agent(), "msg=redirect")
        .expect("a refused redirect is a verdict, not a transport error");

    assert_eq!(class, ResponseClass::Permanent);
    assert!(
        matches!(target_listener.accept(), Err(err) if err.kind() == io::ErrorKind::WouldBlock),
        "a request body must never be replayed to the redirect target"
    );
    let request = requests
        .recv_timeout(Duration::from_secs(5))
        .expect("the original POST is sent once");
    assert_eq!(request.method, "POST");
}

/// Scenario: the collector redirects four times before answering.
///
/// Invariant: the chain is followed and the final response is classified. Four
/// redirects is exactly ureq 2's follow limit.
#[rstest]
fn four_redirects_are_followed(tcp_listener: io::Result<TcpListener>) {
    let class = execute_redirect_chain(tcp_listener.expect("bind ephemeral listener"), 4)
        .expect("four redirects are within the limit");

    assert_eq!(class, ResponseClass::Success);
}

/// Scenario: the collector redirects five times before answering.
///
/// Invariant: the fifth redirect response is a transport error, which is the
/// boundary ureq 2's five-request limit produced. A port that passed ureq 2's
/// number straight to `max_redirects` would follow five instead.
#[rstest]
fn a_fifth_redirect_is_refused(tcp_listener: io::Result<TcpListener>) {
    let err = execute_redirect_chain(tcp_listener.expect("bind ephemeral listener"), 5)
        .expect_err("the fifth redirect response exceeds the limit");

    assert!(
        err.to_lowercase().contains("redirect"),
        "the refusal must name redirects: {err}"
    );
}
