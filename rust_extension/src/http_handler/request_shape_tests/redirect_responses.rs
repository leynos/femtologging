//! Redirect responses that are classified instead of followed.
//!
//! ureq 2 returned several redirect responses to the caller rather than
//! replaying the request: a method it could not replay, a `Location` it could
//! not use, and any status outside the handful it followed. Those responses
//! have to become the same verdicts under ureq 3, which would otherwise follow
//! them or report them as transport errors.

use std::io::{self, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use rstest::rstest;

use super::redirects::{spawn_raw_response, spawn_redirect_server};
use super::{
    AuthConfig, ResponseClass, build_agent, execute, post_config, read_http_request,
    spawn_held_server, spawn_mock_server, tcp_listener, test_agent,
};

/// Answer one attempt with `response` and return its verdict.
fn execute_against_raw_response(
    listener: TcpListener,
    response: Vec<u8>,
) -> Result<ResponseClass, String> {
    let (addr, requests) =
        spawn_raw_response(listener, response).expect("spawn raw response server");
    let config = post_config(format!("http://{addr}/log"), AuthConfig::None, &[]);

    let result = execute(&config, &test_agent(), "msg=redirect");
    requests
        .recv_timeout(Duration::from_secs(5))
        .expect("the original POST is sent once");
    result
}

/// Scenario: a collector answers a POST with a redirect ureq 2 returned
/// instead of following.
///
/// Invariant: the verdict is permanent and the alternative the response names
/// is never requested, exactly as under ureq 2, which returned the redirect
/// response instead of replaying the request body. ureq 3 would follow the
/// `Location`, send a GET, and record the collector's `200` as a delivered
/// record even though the original POST was never accepted. A 307 or 308
/// answering a POST is refused because replaying it would resend the body; a
/// 300 has no replay rule at all.
#[rstest]
#[case::multiple_choices(300)]
#[case::temporary_redirect_of_a_post(307)]
#[case::permanent_redirect_of_a_post(308)]
fn a_redirect_ureq_two_would_not_replay_is_permanent(
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

/// Scenario: the collector answers a redirect that ureq 3 has no usable
/// `Location` to follow.
///
/// Invariant: the verdict is permanent, which is what ureq 2 produced when it
/// returned the redirect response instead of following it. The two cases are a
/// missing header, which ureq 3 raises as `NoLocationHeader`, and a header that
/// is not valid UTF-8, which ureq 2 read as missing and ureq 3 raises as
/// `BadLocationHeader`. Left unhandled, either would become a transport error
/// and the record would be retried until the backoff deadline.
#[rstest]
#[case::missing(b"HTTP/1.1 302 Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec())]
#[case::not_utf8(
    b"HTTP/1.1 302 Found\r\nLocation: http://example.invalid/\x80\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        .to_vec()
)]
fn a_redirect_without_a_usable_location_is_permanent(
    tcp_listener: io::Result<TcpListener>,
    #[case] response: Vec<u8>,
) {
    let class =
        execute_against_raw_response(tcp_listener.expect("bind ephemeral listener"), response)
            .expect("an unusable Location is a verdict, not a transport error");

    assert_eq!(class, ResponseClass::Permanent);
}

/// Scenario: a collector answers 302 with no `Location`, declaring a body it
/// never sends.
///
/// Invariant: the verdict is permanent and does not wait for the body. The
/// agent runs with `max_redirects(0)` and `max_redirects_will_error(false)`, so
/// ureq 3 returns the redirect response instead of calling its
/// `consume_redirect_body` path; nothing reads a redirect body, whether or not
/// there is a `Location` to follow. An attempt that read the declared body
/// would consume the budget and end as a retryable transport error.
#[rstest]
fn a_redirect_without_a_location_does_not_wait_for_its_body(tcp_listener: io::Result<TcpListener>) {
    let listener = tcp_listener.expect("bind ephemeral listener");
    let (addr, answered, release) = spawn_held_server(listener, |stream| {
        read_http_request(stream)?;
        stream.write_all(
            b"HTTP/1.1 302 Found\r\nContent-Length: 1048576\r\nConnection: close\r\n\r\n",
        )
    })
    .expect("spawn held redirect server");
    let budget = Duration::from_secs(5);
    let agent = build_agent(budget, budget);
    let config = post_config(format!("http://{addr}/log"), AuthConfig::None, &[]);

    let started = Instant::now();
    let result = execute(&config, &agent, "msg=redirect");
    let elapsed = started.elapsed();
    let _ = release.send(());

    assert_eq!(
        result,
        Ok(ResponseClass::Permanent),
        "a redirect with no Location is a verdict, not a transport error"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "the body must not be read: a stalled one would consume the {budget:?} budget, took {elapsed:?}"
    );
    answered
        .recv_timeout(Duration::from_secs(5))
        .expect("the redirect response is answered before the client gives up");
}

/// Scenario: a collector answers 307 with a `Location` and a body that stalls.
///
/// Invariant: the refusal is permanent and does not wait for the body, which is
/// what ureq 2 did: it read only the response head, saw a method it could not
/// replay, and returned the response for classification. The attempt also has
/// to finish inside its budget; an attempt that read the body would consume the
/// timeout and end as a retryable transport error instead.
#[rstest]
fn a_stalled_redirect_body_does_not_delay_the_refusal(tcp_listener: io::Result<TcpListener>) {
    let listener = tcp_listener.expect("bind ephemeral listener");
    let target_listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind redirect target");
    let moved_to = format!(
        "http://{}/moved",
        target_listener.local_addr().expect("target address")
    );
    let (addr, answered, release) = spawn_held_server(listener, move |stream| {
        read_http_request(stream)?;
        stream.write_all(
            format!(
                "HTTP/1.1 307 Temporary Redirect\r\nLocation: {moved_to}\r\nContent-Length: 1048576\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
    })
    .expect("spawn held redirect server");
    let budget = Duration::from_secs(5);
    let agent = build_agent(budget, budget);
    let config = post_config(format!("http://{addr}/log"), AuthConfig::None, &[]);

    let started = Instant::now();
    let result = execute(&config, &agent, "msg=redirect");
    let elapsed = started.elapsed();
    let _ = release.send(());

    assert_eq!(
        result,
        Ok(ResponseClass::Permanent),
        "a refused redirect is a verdict, not a transport error"
    );
    assert!(
        elapsed < budget,
        "the body must not be read: a stalled one would consume the {budget:?} budget, took {elapsed:?}"
    );
    answered
        .recv_timeout(Duration::from_secs(5))
        .expect("the redirect response is answered before the client gives up");
}

/// Scenario: the collector names its redirect target with an unencoded space.
///
/// Invariant: the redirect is followed to the percent-encoded path ureq 2
/// produced. ureq 2 resolved `Location` with `url::Url`, which encodes the
/// space; ureq 3 parses an `http::Uri` and would refuse to follow it, turning a
/// deliverable record into a permanent failure.
#[rstest]
fn a_redirect_to_an_unencoded_location_is_followed(tcp_listener: io::Result<TcpListener>) {
    let redirect_listener = tcp_listener.expect("bind ephemeral listener");
    let target_listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind redirect target");
    let (target_addr, target_requests) =
        spawn_mock_server(target_listener, 200).expect("spawn redirect target");
    let (redirect_addr, redirect_requests) = spawn_redirect_server(
        redirect_listener,
        302,
        format!("http://{target_addr}/log ingest"),
    )
    .expect("spawn redirect server");
    let config = post_config(format!("http://{redirect_addr}/log"), AuthConfig::None, &[]);

    let class = execute(&config, &test_agent(), "msg=redirect").expect("the redirect is followed");

    assert_eq!(class, ResponseClass::Success);
    redirect_requests
        .recv_timeout(Duration::from_secs(5))
        .expect("the original request is sent");
    let followed = target_requests
        .recv_timeout(Duration::from_secs(5))
        .expect("the redirect target is reached");
    assert_eq!(followed.path, "/log%20ingest");
}
