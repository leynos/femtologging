//! Contracts for what `FemtoHTTPHandler`'s shutdown deadline bounds.
//!
//! `close_tests.rs` covers the handler side of the contract: that `close`
//! returns inside its budget and says what it did. These cover the worker side,
//! which a bounded `close` alone does not give you. A handler that abandons its
//! worker has still left that worker running, and the point of the signal is
//! that the abandoned worker stops promptly and stops *there*, rather than
//! spending a backoff deadline and an unbounded queue behind the caller's back.
//!
//! Three contracts, one per way the worker could outlive its deadline:
//!
//! - a server that answers 429 or 503 forever must not hold the worker past
//!   the budget. Each retry is one request, and the deadline is what stops
//!   the loop rather than the backoff policy running out;
//! - a close that lands during a retry delay must be observed there, not after
//!   it. The signal's `wait` is the interruptible stand-in for the sleep this
//!   used to be, and the worker is known to be inside the delay before the
//!   close is issued;
//! - a queue that outlasts the deadline must be discarded rather than sent.
//!   The count is warned about, because records disappearing silently is the
//!   failure this whole change exists to make visible.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use rstest::rstest;

use super::{send_info_record, tcp_listener};
use crate::http_handler::FemtoHTTPHandler;
use crate::http_handler::config::{HTTPHandlerConfig, HTTPMethod};
use crate::socket_handler::BackoffPolicy;

/// How long a test waits for the worker to reach the server or the delay.
const ARRIVAL_ALLOWANCE: Duration = Duration::from_secs(10);
/// The handler's flush and shutdown budget.
const CLOSE_BUDGET: Duration = Duration::from_millis(200);
/// The shutdown budget plus 500ms for scheduling.
const CLOSE_ALLOWANCE: Duration = Duration::from_millis(700);
/// The backoff deadline the worker would run to if nothing stopped it.
const BACKOFF_DEADLINE: Duration = Duration::from_secs(30);

/// Spawn a server that answers every request with `status`, for as long as the
/// test runs, and reports each request it answered.
///
/// Unlike `spawn_retry_server`, the status list never empties: a handler under
/// test retries until something other than the server stops it, which is the
/// condition these contracts are about.
fn spawn_perpetual_server(
    listener: TcpListener,
    status: u16,
) -> io::Result<(SocketAddr, mpsc::Receiver<()>)> {
    let addr = listener.local_addr()?;
    let (answered_tx, answered_rx) = mpsc::channel();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            // The client writes its request first; reading it back is what
            // lets it settle into waiting for the response, so the response is
            // what the worker is actually blocked on.
            let mut sink = [0_u8; 1024];
            let _ = std::io::Read::read(&mut stream, &mut sink);
            let response = format!("HTTP/1.1 {status} X\r\nContent-Length: 0\r\n\r\n");
            if stream.write_all(response.as_bytes()).is_err() {
                break;
            }
            if answered_tx.send(()).is_err() {
                break;
            }
        }
    });
    Ok((addr, answered_rx))
}

/// Read one request and hold the connection open without answering it.
///
/// The worker's request timeout then keeps the record in flight past the close
/// deadline, leaving queued records for the worker to count after it aborts.
///
/// # Examples
///
/// ```ignore
/// let (addr, received) = spawn_stalled_server(listener)?;
/// received.recv_timeout(Duration::from_secs(2))?;
/// ```
fn spawn_stalled_server(listener: TcpListener) -> io::Result<(SocketAddr, mpsc::Receiver<()>)> {
    let addr = listener.local_addr()?;
    let (request_tx, request_rx) = mpsc::channel();
    thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        if super::read_http_request(&mut stream).is_err() {
            return;
        }
        let _ = request_tx.send(());
        let mut sink = [0_u8; 1];
        while matches!(stream.read(&mut sink), Ok(read) if read > 0) {}
    });
    Ok((addr, request_rx))
}

/// Build a handler with a short budget and a backoff deadline far longer than
/// the test is willing to wait.
fn build_retrying_handler(addr: SocketAddr, capacity: usize, delay: Duration) -> FemtoHTTPHandler {
    let config = HTTPHandlerConfig {
        url: format!("http://{addr}/log"),
        method: HTTPMethod::POST,
        capacity,
        connect_timeout: CLOSE_BUDGET,
        write_timeout: CLOSE_BUDGET,
        backoff: BackoffPolicy {
            base: delay,
            cap: delay,
            reset_after: Duration::from_secs(60),
            deadline: BACKOFF_DEADLINE,
        },
        ..Default::default()
    };
    FemtoHTTPHandler::with_config(config)
}

/// Scenario: the endpoint answers a retryable status to every request.
///
/// Invariant: `close` returns inside its budget, and the worker stops retrying
/// rather than running out its backoff deadline. The retry loop used to be
/// bounded only by that deadline, which the configuration sets independently of
/// the flush budget: a rate-limited or failing collector meant a close that
/// waited for however long the policy said, not for the budget the caller
/// chose.
///
/// Mutation proof: removing the `is_shutting_down` check from `should_retry`
/// leaves the loop running to `BACKOFF_DEADLINE`, and this test fails once
/// close exceeds `CLOSE_ALLOWANCE`.
#[rstest]
#[case(429)]
#[case(503)]
#[serial_test::serial]
fn a_retryable_endpoint_does_not_outlast_the_close_budget(
    tcp_listener: io::Result<TcpListener>,
    #[case] status: u16,
) {
    let tcp_listener = tcp_listener.expect("bind ephemeral listener");
    let (addr, answered) = spawn_perpetual_server(tcp_listener, status).expect("spawn server");
    crate::handlers::file::test_support::install_test_logger();
    let mut handler = build_retrying_handler(addr, 16, Duration::from_millis(50));
    send_info_record(&handler, "rate limited").expect("record should be queued");
    answered
        .recv_timeout(ARRIVAL_ALLOWANCE)
        .expect("the worker should reach the retryable endpoint");

    let started = Instant::now();
    handler.close();
    let elapsed = started.elapsed();

    assert!(
        elapsed < CLOSE_ALLOWANCE,
        "close exceeded the retryable {status} response deadline after {elapsed:?}; allowed \
         {CLOSE_ALLOWANCE:?}, backoff was {BACKOFF_DEADLINE:?}"
    );
}

/// Scenario: a close lands while the worker is inside a retry delay.
///
/// Invariant: the worker notices there, not when the delay elapses. The retry
/// sleep used to be `thread::sleep`, which no shutdown can interrupt, so the
/// close was observed only after a delay whose length the backoff policy chose.
/// The five-second delay keeps the worker there until close starts, and the
/// pause hook holds it at the wait boundary until the test observes shutdown.
/// This makes the ordering deterministic even when the test thread is delayed.
#[rstest]
#[serial_test::serial]
fn a_close_during_a_retry_delay_is_answered_from_it(tcp_listener: io::Result<TcpListener>) {
    let tcp_listener = tcp_listener.expect("bind ephemeral listener");
    let (addr, answered) = spawn_perpetual_server(tcp_listener, 503).expect("spawn server");
    crate::handlers::file::test_support::install_test_logger();
    let mut handler = build_retrying_handler(addr, 16, Duration::from_secs(5));
    let (delays, resume_delay) = handler.pause_next_wait();
    send_info_record(&handler, "retrying").expect("record should be queued");
    answered
        .recv_timeout(ARRIVAL_ALLOWANCE)
        .expect("the worker should reach the server");
    delays
        .recv_timeout(ARRIVAL_ALLOWANCE)
        .expect("the worker should enter a retry delay");

    let signal = handler.signal_for_test();
    let (close_done_tx, close_done_rx) = mpsc::channel();
    let close_thread = thread::spawn(move || {
        let started = Instant::now();
        handler.close();
        close_done_tx
            .send(started.elapsed())
            .expect("the test should still be waiting for close");
    });

    let shutdown_wait_started = Instant::now();
    while !signal.is_shutting_down() {
        assert!(
            shutdown_wait_started.elapsed() < ARRIVAL_ALLOWANCE,
            "close should publish shutdown before the paused wait resumes"
        );
        thread::yield_now();
    }
    resume_delay
        .send(())
        .expect("resume the worker after shutdown is visible");

    let elapsed = close_done_rx
        .recv_timeout(CLOSE_ALLOWANCE)
        .expect("close should finish within its deadline and scheduling margin");
    close_thread
        .join()
        .expect("the close thread should not panic");
    assert!(
        elapsed < CLOSE_ALLOWANCE,
        "close must not wait out a retry delay: took {elapsed:?}, allowed \
         {CLOSE_ALLOWANCE:?}"
    );
    assert!(
        matches!(answered.try_recv(), Err(mpsc::TryRecvError::Empty)),
        "closing during backoff must stop this record from being retried"
    );
    let logged = crate::handlers::file::test_support::take_logged_messages();
    let abandoned: Vec<_> = logged
        .iter()
        .filter(|entry| entry.message.contains(ABANDON_MARKER))
        .collect();
    assert!(
        abandoned.is_empty(),
        "a worker interrupted during a retry delay must acknowledge the \
         shutdown rather than be abandoned; close logged {abandoned:?}"
    );
}

/// The message `abandon_worker` logs, and the marker this file turns on.
const ABANDON_MARKER: &str = "abandoning it rather than blocking the caller";

/// Scenario: the queue holds more than the deadline can deliver, and the
/// endpoint answers 5xx to everything.
///
/// Invariant: the records still queued when the deadline passes are discarded
/// rather than sent, and the handler says how many. The drain this replaced
/// processed the whole backlog however long that took, so a queue of records
/// against a failing endpoint was a close that ran for one request per record,
/// each retrying to the backoff deadline.
///
/// The assertion is the warning, not the elapsed time: a fast close that
/// silently dropped the records would satisfy any clock, and silent loss is the
/// failure this change exists to make visible. The elapsed assertion is there
/// to stop the test passing on a handler that sent them all quickly.
#[rstest]
#[serial_test::serial]
fn records_left_at_the_deadline_are_discarded_and_counted(tcp_listener: io::Result<TcpListener>) {
    let tcp_listener = tcp_listener.expect("bind ephemeral listener");
    let (addr, request_received) = spawn_stalled_server(tcp_listener).expect("spawn server");
    crate::handlers::file::test_support::install_test_logger();
    let mut handler = build_retrying_handler(addr, 64, Duration::from_millis(50));
    // Enough records that the deadline cannot deliver them all at one request
    // per `CLOSE_BUDGET`, and queued before the close so they are the backlog
    // the drain has to decide about.
    for index in 0..32 {
        send_info_record(&handler, &format!("backlog {index}")).expect("record should be queued");
    }
    request_received
        .recv_timeout(ARRIVAL_ALLOWANCE)
        .expect("the worker should reach the stalled request");

    let started = Instant::now();
    handler.close();
    let elapsed = started.elapsed();

    assert!(
        elapsed < CLOSE_ALLOWANCE,
        "close must not work through an unbounded backlog: took {elapsed:?}, \
         allowed {CLOSE_ALLOWANCE:?}"
    );
    let logged = crate::handlers::file::test_support::wait_for_message(
        DISCARD_MARKER,
        Duration::from_secs(3),
    );
    assert!(
        logged.iter().any(|entry| {
            entry.message.contains("discarded ") && entry.message.contains(DISCARD_MARKER)
        }),
        "a shutdown that discards queued records must say how many; close \
         logged {logged:?}"
    );
}

/// The message `warn_discarded` logs, and the marker the backlog test turns on.
const DISCARD_MARKER: &str = "queued records at its shutdown deadline";
