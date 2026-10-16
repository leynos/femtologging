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
//! - a server that answers 429 forever must not hold the worker past the
//!   budget. Each retry is one request, and the deadline is what stops the
//!   loop rather than the backoff policy running out;
//! - a close that lands during a retry delay must be observed there, not after
//!   it. The signal's `wait` is the interruptible stand-in for the sleep this
//!   used to be, and the worker is known to be inside the delay before the
//!   close is issued;
//! - a queue that outlasts the deadline must be discarded rather than sent.
//!   The count is warned about, because records disappearing silently is the
//!   failure this whole change exists to make visible.

use std::io::{self, Write};
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
/// What a close is allowed to take. Well above `CLOSE_BUDGET` so a loaded host
/// cannot fail it, and well below the backoff deadline so a worker that ran to
/// it cannot pass.
const CLOSE_ALLOWANCE: Duration = Duration::from_secs(3);
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

/// Build a handler with a short budget and a backoff deadline far longer than
/// the test is willing to wait.
fn build_retrying_handler(addr: SocketAddr, capacity: usize) -> FemtoHTTPHandler {
    let config = HTTPHandlerConfig {
        url: format!("http://{addr}/log"),
        method: HTTPMethod::POST,
        capacity,
        connect_timeout: CLOSE_BUDGET,
        write_timeout: CLOSE_BUDGET,
        backoff: BackoffPolicy {
            base: Duration::from_millis(10),
            cap: Duration::from_millis(50),
            reset_after: Duration::from_secs(60),
            deadline: BACKOFF_DEADLINE,
        },
        ..Default::default()
    };
    FemtoHTTPHandler::with_config(config)
}

/// Scenario: the endpoint answers 429 to every request the worker makes.
///
/// Invariant: `close` returns inside its budget, and the worker stops retrying
/// rather than running out its backoff deadline. The retry loop used to be
/// bounded only by that deadline, which the configuration sets independently of
/// the flush budget: a rate-limited collector meant a close that waited for
/// however long the policy said, not for the budget the caller chose.
///
/// Mutation proof: removing the `is_shutting_down` check from `should_retry`
/// leaves the loop running to `BACKOFF_DEADLINE`, and this test fails at three
/// seconds.
#[rstest]
#[serial_test::serial]
fn a_rate_limited_endpoint_does_not_outlast_the_close_budget(
    tcp_listener: io::Result<TcpListener>,
) {
    let tcp_listener = tcp_listener.expect("bind ephemeral listener");
    let (addr, answered) = spawn_perpetual_server(tcp_listener, 429).expect("spawn server");
    crate::handlers::file::test_support::install_test_logger();
    let mut handler = build_retrying_handler(addr, 16);
    send_info_record(&handler, "rate limited").expect("record should be queued");
    answered
        .recv_timeout(ARRIVAL_ALLOWANCE)
        .expect("the worker should reach the server");

    let started = Instant::now();
    handler.close();
    let elapsed = started.elapsed();

    assert!(
        elapsed < CLOSE_ALLOWANCE,
        "close must not outlast its budget against a perpetual 429: took \
         {elapsed:?}, allowed {CLOSE_ALLOWANCE:?}, the retry loop would run \
         for {BACKOFF_DEADLINE:?}"
    );
}

/// Scenario: a close lands while the worker is inside a retry delay.
///
/// Invariant: the worker notices there, not when the delay elapses. The retry
/// sleep used to be `thread::sleep`, which no shutdown can interrupt, so the
/// close was observed only after a delay whose length the backoff policy chose.
/// The delay here is capped at 50ms, so the assertion is not about the wait
/// being long in absolute terms: it is that the close is answered at all, which
/// a worker asleep in `thread::sleep` cannot do until the sleep ends.
///
/// The wait is observed rather than guessed. `wait_notification` fires when the
/// worker enters the sleep, so the close is issued against a worker known to be
/// inside it; a sleep long enough to assume that would measure the host.
#[rstest]
#[serial_test::serial]
fn a_close_during_a_retry_delay_is_answered_from_it(tcp_listener: io::Result<TcpListener>) {
    let tcp_listener = tcp_listener.expect("bind ephemeral listener");
    let (addr, answered) = spawn_perpetual_server(tcp_listener, 503).expect("spawn server");
    crate::handlers::file::test_support::install_test_logger();
    let mut handler = build_retrying_handler(addr, 16);
    let delays = handler.wait_notification();
    send_info_record(&handler, "retrying").expect("record should be queued");
    answered
        .recv_timeout(ARRIVAL_ALLOWANCE)
        .expect("the worker should reach the server");
    delays
        .recv_timeout(ARRIVAL_ALLOWANCE)
        .expect("the worker should enter a retry delay");

    assert_close_cooperates(&mut handler);
}

/// Close `handler` and assert the worker answered rather than being abandoned.
///
/// A worker that wakes for the signal acknowledges the shutdown, so `close`
/// joins it and warns about nothing. A worker asleep in the old
/// `thread::sleep` misses the signal entirely, and the only way out of the
/// close is the abandonment path - which is what this asserts did not happen.
fn assert_close_cooperates(handler: &mut FemtoHTTPHandler) {
    let started = Instant::now();
    handler.close();
    let elapsed = started.elapsed();

    assert!(
        elapsed < CLOSE_ALLOWANCE,
        "close must not wait out a retry delay: took {elapsed:?}, allowed \
         {CLOSE_ALLOWANCE:?}"
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
    let (addr, answered) = spawn_perpetual_server(tcp_listener, 503).expect("spawn server");
    crate::handlers::file::test_support::install_test_logger();
    let mut handler = build_retrying_handler(addr, 64);
    // Enough records that the deadline cannot deliver them all at one request
    // per `CLOSE_BUDGET`, and queued before the close so they are the backlog
    // the drain has to decide about.
    for index in 0..32 {
        send_info_record(&handler, &format!("backlog {index}")).expect("record should be queued");
    }
    answered
        .recv_timeout(ARRIVAL_ALLOWANCE)
        .expect("the worker should reach the server");

    let started = Instant::now();
    handler.close();
    let elapsed = started.elapsed();

    assert!(
        elapsed < CLOSE_ALLOWANCE,
        "close must not work through an unbounded backlog: took {elapsed:?}, \
         allowed {CLOSE_ALLOWANCE:?}"
    );
    let logged = crate::handlers::file::test_support::take_logged_messages();
    assert!(
        logged
            .iter()
            .any(|entry| entry.message.contains(DISCARD_MARKER)),
        "a shutdown that discards queued records must say how many; close \
         logged {logged:?}"
    );
}

/// The message `warn_discarded` logs, and the marker the backlog test turns on.
const DISCARD_MARKER: &str = "queued records at its shutdown deadline";
