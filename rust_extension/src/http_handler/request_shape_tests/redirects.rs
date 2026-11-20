//! Redirect contracts: the follow limit and credential stripping.
//!
//! The fixtures serve redirect chains on loopback. They live beside the request
//! shape tests because they share the same server helpers, and in their own
//! files so each stays readable. The sibling `redirect_responses` module covers
//! the redirects that are classified rather than followed.

use std::io::{self, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use rstest::rstest;

use super::{
    AuthConfig, CapturedRequest, ResponseClass, execute, header_values, post_config,
    read_http_request, spawn_mock_server, tcp_listener, test_agent,
};
use crate::http_handler::transport::execute as execute_attempt;

/// Spawn a server that answers its one request with `response` verbatim.
///
/// Some redirect contracts are about responses that cannot be built with the
/// normal helper: a redirect with no `Location`, or one whose `Location` is not
/// valid UTF-8.
pub(super) fn spawn_raw_response(
    listener: TcpListener,
    response: Vec<u8>,
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
        let _ = stream.write_all(&response);
        let _ = capture_tx.send(request);
    });
    Ok((addr, capture_rx))
}

/// Spawn a server that answers its one request with `status` and `location`.
pub(super) fn spawn_redirect_server(
    listener: TcpListener,
    status: u16,
    location: String,
) -> io::Result<(SocketAddr, mpsc::Receiver<CapturedRequest>)> {
    let response = format!(
        "HTTP/1.1 {status} Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    spawn_raw_response(listener, response.into_bytes())
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
/// has used its redirects; the request after that is answered 200, and every
/// answer is held back by `delay` so a test can spend the attempt's budget
/// inside the chain. A client that refuses a redirect stops opening
/// connections, so the accept loop gives up after a short idle period rather
/// than block forever on a connection that is never coming - a hung fixture
/// would hide the regression the tests are looking for.
///
/// The returned counter records how many requests the server read, which is
/// how a test tells a chain that stopped on time from one that ran to the end.
fn spawn_redirect_chain(
    listener: TcpListener,
    redirects: usize,
    delay: Duration,
) -> io::Result<(SocketAddr, thread::JoinHandle<()>, Arc<AtomicUsize>)> {
    let addr = listener.local_addr()?;
    listener.set_nonblocking(true)?;
    let served = Arc::new(AtomicUsize::new(0));
    let served_by_server = Arc::clone(&served);
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
            served_by_server.fetch_add(1, Ordering::SeqCst);
            thread::sleep(delay);
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
    Ok((addr, handle, served))
}

/// Run one attempt against a chain of `redirects` redirects.
fn execute_redirect_chain(
    listener: TcpListener,
    redirects: usize,
) -> Result<ResponseClass, String> {
    let (addr, server, _served) =
        spawn_redirect_chain(listener, redirects, Duration::ZERO).expect("spawn redirect chain");
    let config = post_config(format!("http://{addr}/hop0"), AuthConfig::None, &[]);

    let result = execute(&config, &test_agent(), "msg=redirect");
    server.join().expect("redirect chain server should finish");
    result
}

/// Scenario: the collector redirects to a second listener, and the request
/// carries the configured auth, a cookie, and a custom credential header.
///
/// Invariant: the redirect is followed, but neither `Authorization` nor
/// `Cookie` is replayed at the target. ureq 2 dropped both, and ureq 3's
/// `RedirectAuthHeaders` setting covers only `Authorization`, so the chain
/// removes the cookie itself. A custom credential header is still forwarded;
/// that is also what ureq 2 did, and the documentation states the distinction
/// rather than claiming every credential is stripped.
#[rstest]
fn a_redirect_strips_credentials_and_forwards_custom_headers(
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
        &[
            ("cookie", "session=redirect-secret"),
            ("x-api-key", "custom-credential"),
        ],
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
    assert_eq!(
        header_values(&first, "cookie"),
        ["session=redirect-secret"],
        "the first request carries the configured cookie"
    );
    let second = target_requests
        .recv_timeout(Duration::from_secs(5))
        .expect("the redirect target should be reached");
    assert_eq!(second.path, "/collected");
    assert!(
        header_values(&second, "authorization").is_empty(),
        "a redirect must not replay the Authorization header: {:?}",
        header_values(&second, "authorization")
    );
    assert!(
        header_values(&second, "cookie").is_empty(),
        "a redirect must not replay a cookie: {:?}",
        header_values(&second, "cookie")
    );
    assert_eq!(
        header_values(&second, "x-api-key"),
        ["custom-credential"],
        "a custom header is outside RedirectAuthHeaders and is forwarded"
    );
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

/// Scenario: every hop answers slowly, and the attempt's budget cannot cover
/// the whole chain.
///
/// Invariant: the budget bounds the attempt rather than each hop. Giving every
/// hop a fresh timeout would let a four-redirect chain occupy five times the
/// budget; with one deadline the chain stops inside it. During shutdown that
/// budget is the handler's remaining allowance, so this is also the guarantee
/// that a redirect cannot hold the worker past its deadline.
#[rstest]
fn one_deadline_bounds_the_whole_redirect_chain(tcp_listener: io::Result<TcpListener>) {
    let delay = Duration::from_millis(250);
    let budget = Duration::from_millis(400);
    let (addr, server, served) =
        spawn_redirect_chain(tcp_listener.expect("bind ephemeral listener"), 4, delay)
            .expect("spawn delayed redirect chain");
    let config = post_config(format!("http://{addr}/hop0"), AuthConfig::None, &[]);

    let started = Instant::now();
    let result = execute_attempt(&config, &test_agent(), "msg=redirect", budget);
    let elapsed = started.elapsed();
    server.join().expect("redirect chain server should finish");

    assert!(
        result.is_err(),
        "a chain that outlasts its budget is a transport failure, not a verdict"
    );
    assert!(
        elapsed < Duration::from_millis(900),
        "the whole chain must stop near its {budget:?} budget, not restart it per hop: took {elapsed:?}"
    );
    assert!(
        served.load(Ordering::SeqCst) < 5,
        "the chain must stop before its last hop is requested"
    );
}

/// Scenario: a hop holds its answer for far longer than the attempt's budget.
///
/// Invariant: the hop's own request is narrowed to the time left, so the
/// attempt ends near its budget. Rechecking the clock only between hops would
/// still let the hop in flight run for the agent's whole timeout.
#[rstest]
fn a_slow_hop_cannot_outlast_the_attempt_budget(tcp_listener: io::Result<TcpListener>) {
    let delay = Duration::from_secs(1);
    let budget = Duration::from_millis(300);
    let (addr, server, _served) =
        spawn_redirect_chain(tcp_listener.expect("bind ephemeral listener"), 1, delay)
            .expect("spawn delayed redirect chain");
    let config = post_config(format!("http://{addr}/hop0"), AuthConfig::None, &[]);

    let started = Instant::now();
    let result = execute_attempt(&config, &test_agent(), "msg=redirect", budget);
    let elapsed = started.elapsed();
    server.join().expect("redirect chain server should finish");

    assert!(
        result.is_err(),
        "a hop that outlasts the budget is a transport failure"
    );
    assert!(
        elapsed < Duration::from_millis(900),
        "the hop must inherit the {budget:?} budget rather than the agent's timeout: took {elapsed:?}"
    );
}
