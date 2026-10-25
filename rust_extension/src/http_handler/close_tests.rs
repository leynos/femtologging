//! Contracts for `FemtoHTTPHandler`'s close path.
//!
//! Split out of `tests.rs`, which reached the 400-line module limit. The
//! stalled-request and full-queue tests share a fixture that accepts a request
//! and never answers it.
//!
//! These tests cover three close outcomes. A stalled request is closed inside
//! the shutdown budget, either cooperatively or by detaching the worker, and is
//! not retried; a full command queue still lets close return inside its budget;
//! and a healthy worker drains its records and is joined.
//!
//! Included with `#[path]` from `tests.rs`, the way
//! `response_classification_tests.rs` already is, so both files stay inside
//! the limit and the helpers above remain in scope.

use std::io::{self, Read};
use std::net::{SocketAddr, TcpListener};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use rstest::rstest;

use super::{build_http_handler, send_info_record, spawn_retry_server, tcp_listener};
use crate::http_handler::FemtoHTTPHandler;
use crate::http_handler::config::{HTTPHandlerConfig, HTTPMethod};

/// Spawn a server that accepts connections, reads each request, and never
/// replies.
///
/// A worker talking to it blocks in `ureq` until its request timeout expires.
/// The close test waits for the request to arrive before closing, so it
/// exercises an in-flight operation rather than relying on a scheduling race.
///
/// The accepted streams are held rather than dropped: closing them would let
/// `ureq` fail immediately and end the very block this fixture exists to
/// create.
///
/// The receiver it returns yields once per request read. A test waits on it
/// rather than sleeping, so the worker is known to be inside its request, and
/// deaf to its command channel, before the test goes on; a sleep would either
/// slow the test or let a loaded host close the handler first.
fn spawn_silent_server(listener: TcpListener) -> io::Result<(SocketAddr, mpsc::Receiver<()>)> {
    let addr = listener.local_addr()?;
    let (read_tx, read_rx) = mpsc::channel();
    thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            // Read what the client sends so it completes its write and settles
            // into waiting for a response that never comes.
            let mut sink = [0_u8; 1024];
            let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
            let _ = stream.read(&mut sink);
            held.push(stream);
            let _ = read_tx.send(());
        }
    });
    Ok((addr, read_rx))
}

/// How long a test waits for the worker to reach the silent server.
const REQUEST_ALLOWANCE: Duration = Duration::from_secs(10);

/// How long the worker is kept busy, and so how long an unbounded join waits.
const BUSY_WINDOW: Duration = Duration::from_secs(10);
/// The handler's flush and shutdown budget.
const CLOSE_BUDGET: Duration = Duration::from_millis(200);
/// What `close` is allowed to take. Generous against `CLOSE_BUDGET` so a
/// loaded host cannot fail it, and far below `BUSY_WINDOW` so an unbounded
/// join cannot pass it.
const CLOSE_ALLOWANCE: Duration = Duration::from_millis(700);

/// Build a handler pointed at a server that never replies, with a command
/// queue holding `capacity` entries.
fn build_stuck_handler(addr: SocketAddr, capacity: usize) -> FemtoHTTPHandler {
    use crate::socket_handler::BackoffPolicy;

    let config = HTTPHandlerConfig {
        url: format!("http://{addr}/log"),
        method: HTTPMethod::POST,
        capacity,
        connect_timeout: CLOSE_BUDGET,
        write_timeout: CLOSE_BUDGET,
        backoff: BackoffPolicy {
            base: Duration::from_millis(10),
            cap: Duration::from_millis(20),
            reset_after: Duration::from_secs(60),
            deadline: BUSY_WINDOW,
        },
        ..Default::default()
    };
    FemtoHTTPHandler::with_config(config)
}

/// Scenario: the worker is stuck mid-request when the handler is closed.
///
/// Invariant: close stays inside its budget whether the request finishes
/// cooperatively or the deadline expires and the worker is detached. Neither
/// outcome may start another attempt for this record.
///
/// `BUSY_WINDOW` is ten seconds, and the assertion allows the budget plus
/// 500ms for scheduling: enough room for a loaded host, and far short of the
/// window an unbounded join would wait out.
///
/// Proved by reverting: with the bound removed, this fails with an elapsed
/// time around the ten-second window. The close outcome is logged, and the
/// absence of a second request proves that cancellation stopped this record's
/// retries.
///
/// `#[serial]` because this module and `drain_tests.rs` share one process-wide
/// log capture. The stalled test accepts either close warning, while the
/// healthy-close test asserts that abandonment was not reported.
#[rstest]
#[serial_test::serial]
fn close_cancels_a_stalled_request_within_its_budget(tcp_listener: io::Result<TcpListener>) {
    let tcp_listener = tcp_listener.expect("bind ephemeral listener");
    let (addr, requests) = spawn_silent_server(tcp_listener).expect("spawn silent server");
    crate::handlers::file::test_support::install_test_logger();
    let mut handler = build_stuck_handler(addr, 16);
    send_info_record(&handler, "stuck").expect("record should be queued");

    // Wait until the worker is inside its request, so the shutdown command
    // arrives at a worker that cannot read it. Without this the worker might
    // acknowledge before it ever sends, and the test would pass without
    // exercising the path it names.
    requests
        .recv_timeout(REQUEST_ALLOWANCE)
        .expect("the worker should reach the silent server");

    let logged = close_within_budget(&mut handler);
    assert!(
        logged.iter().any(|entry| {
            entry.message.contains(SHUTDOWN_DROP_MARKER) || entry.message.contains(ABANDON_MARKER)
        }),
        "close should report the cancelled record or worker detachment: {logged:?}"
    );
    assert!(
        requests.recv_timeout(Duration::from_millis(300)).is_err(),
        "the cancelled record must not be retried after close"
    );
}

/// Close `handler`, asserting it returned within the deadline and margin.
fn close_within_budget(
    handler: &mut FemtoHTTPHandler,
) -> Vec<crate::handlers::file::test_support::CapturedLog> {
    let started = Instant::now();
    handler.close();
    let elapsed = started.elapsed();

    assert!(
        elapsed < CLOSE_ALLOWANCE,
        "close must not outlast its flush budget and scheduling margin: \
         took {elapsed:?}, allowed {CLOSE_ALLOWANCE:?}"
    );
    crate::handlers::file::test_support::take_logged_messages()
}

/// Scenario: the worker is stuck mid-request and its command queue is full
/// when the handler is closed.
///
/// Invariant: `close` still returns within its budget. The shutdown command
/// has to be queued before any acknowledgement can be awaited, and on a full
/// bounded channel a plain send blocks until the worker next reads, which a
/// stuck worker does not do for the whole of its backoff deadline. The bound
/// on the acknowledgement alone left that wait in front of it.
///
/// A queue of one entry makes the state reachable deterministically: the
/// worker holds the first record in its request, and the second fills the
/// queue.
///
/// Mutation proof: sending the shutdown command with a plain `send` fails
/// this test at roughly the ten-second window, and no other.
#[rstest]
#[serial_test::serial]
fn close_returns_within_its_budget_when_the_queue_is_full(tcp_listener: io::Result<TcpListener>) {
    let tcp_listener = tcp_listener.expect("bind ephemeral listener");
    let (addr, requests) = spawn_silent_server(tcp_listener).expect("spawn silent server");
    crate::handlers::file::test_support::install_test_logger();
    let mut handler = build_stuck_handler(addr, 1);
    send_info_record(&handler, "held in the request").expect("first record should be queued");
    requests
        .recv_timeout(REQUEST_ALLOWANCE)
        .expect("the worker should reach the silent server");
    send_info_record(&handler, "fills the queue").expect("second record should be queued");
    assert!(
        send_info_record(&handler, "overflows").is_err(),
        "the queue should be full, or the shutdown send is not under test"
    );

    close_within_budget(&mut handler);
}

/// The message `abandon_worker` logs; the stalled and healthy close tests use it.
const ABANDON_MARKER: &str = "abandoning it rather than blocking the caller";
/// The warning emitted when an in-flight request ends after shutdown begins.
const SHUTDOWN_DROP_MARKER: &str = "FemtoHTTPHandler dropped 1 records during shutdown";

/// Scenario: the worker answers the shutdown request promptly.
///
/// Invariant: `close` joins it, and says nothing about abandoning anything.
///
/// This is the narrow half, and it was written because the sufficient half
/// did not need it. Replacing the whole decision with an unconditional
/// `abandon_worker` passes every other test in this crate, including the test
/// above: skipping a join is invisible to a caller that was not waiting on it,
/// and the acknowledgement already arrives after the worker has drained its
/// queue, so nothing observable is left for the join to guarantee. A bound
/// that is never taken is not a bound, and nothing would have failed.
///
/// The one thing the two paths do differ in is what they say. Abandoning a
/// worker is a warning, because it leaves a thread running past the handler
/// that owned it, and a handler that warns about that when it did not happen
/// is worse than one that says nothing at all. So the assertion is on the
/// message: a healthy close must not claim to have abandoned its worker.
///
/// Mutation proof: replacing the match in `close` with an unconditional
/// `abandon_worker` fails this test on the warning it then emits.
#[rstest]
#[serial_test::serial]
fn a_close_whose_worker_answers_does_not_report_abandoning_it(
    tcp_listener: io::Result<TcpListener>,
) {
    let tcp_listener = tcp_listener.expect("bind ephemeral listener");
    let (addr, requests) =
        spawn_retry_server(tcp_listener, vec![200, 200]).expect("spawn mock server");
    crate::handlers::file::test_support::install_test_logger();
    let mut handler = build_http_handler(addr);
    send_info_record(&handler, "healthy record one").expect("first record should be queued");
    send_info_record(&handler, "healthy record two").expect("second record should be queued");

    handler.close();

    let captured: Vec<_> = (0..2)
        .map(|_| {
            requests
                .recv_timeout(Duration::from_secs(2))
                .expect("graceful close should deliver each queued record")
        })
        .collect();
    assert!(
        captured
            .iter()
            .any(|request| request.body.contains("msg=healthy+record+one")),
        "the first record should reach the collector: {captured:?}"
    );
    assert!(
        captured
            .iter()
            .any(|request| request.body.contains("msg=healthy+record+two")),
        "the second record should reach the collector: {captured:?}"
    );

    let abandoned: Vec<_> = crate::handlers::file::test_support::take_logged_messages()
        .into_iter()
        .filter(|entry| entry.message.contains(ABANDON_MARKER))
        .collect();
    assert!(
        abandoned.is_empty(),
        "a worker that acknowledged shutdown must be joined, not abandoned; \
         close logged {abandoned:?}"
    );
}
