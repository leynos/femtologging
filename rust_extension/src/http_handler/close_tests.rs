//! Contracts for `FemtoHTTPHandler`'s close path.
//!
//! Split out of `tests.rs`, which reached the 400-line module limit, and split
//! here rather than anywhere else because these two tests share a fixture
//! nothing else uses: a server that accepts a request and never answers it.
//!
//! The first two are the two halves of one rule. One asserts that `close`
//! returns inside its flush budget, and says it abandoned the worker, when the
//! worker never acknowledges; the other that a worker which does acknowledge
//! is joined rather than abandoned. The second exists because the first does
//! not need it: abandoning unconditionally passed every other test in the
//! crate. The third covers the step before the acknowledgement: queueing the
//! shutdown command on a channel the stuck worker has left full.
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

use super::{build_http_handler, send_info_record, spawn_mock_server, tcp_listener};
use crate::http_handler::FemtoHTTPHandler;
use crate::http_handler::config::{HTTPHandlerConfig, HTTPMethod};

/// Spawn a server that accepts connections, reads each request, and never
/// replies.
///
/// A worker talking to it blocks in `ureq` until the agent's own timeout
/// expires, then retries, and keeps doing so until its backoff deadline. It is
/// therefore busy, and not reading its command channel, for a window the test
/// chooses. That is what makes the close path's behaviour on a worker that
/// never acknowledges observable rather than incidental.
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
const CLOSE_ALLOWANCE: Duration = Duration::from_secs(3);

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

/// Scenario: the worker is stuck mid-request when the handler is closed, so it
/// never acknowledges the shutdown.
///
/// Invariant: `close` returns within its own flush budget rather than waiting
/// for the worker. The shutdown request is bounded by `flush_timeout`, but the
/// join that followed it was not, so a timed-out acknowledgement still blocked
/// the caller for as long as the worker took to finish: here, the whole
/// backoff deadline. `close` runs from `Drop`, so a logger being torn down
/// could hold a thread for a window nothing in the configuration describes.
///
/// The two figures are deliberately far apart. `CLOSE_BUDGET` is 200ms and
/// `BUSY_WINDOW` is ten seconds, and the assertion sits at three seconds:
/// fifteen times the budget, so a loaded host cannot fail it, and a third of
/// the window, so an unbounded join cannot pass it. A test whose verdict
/// depends on how busy the machine is asserts the machine.
///
/// Proved by reverting: with the bound removed, this fails with an elapsed
/// time around the ten-second window. The abandonment warning is asserted
/// too, since a quick return alone could be a join that happened to be fast.
///
/// `#[serial]` because all three tests here read one process-wide log
/// capture: two assert the abandonment message is present, and the healthy
/// close asserts it is absent, and these are the only tests in the binary
/// that emit it.
#[rstest]
#[serial_test::serial(http_close)]
fn close_returns_within_its_budget_when_the_worker_never_acknowledges(
    tcp_listener: io::Result<TcpListener>,
) {
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

    assert_close_abandons_within_budget(&mut handler);
}

/// Close `handler`, asserting it returned inside the allowance and said it
/// abandoned the worker.
///
/// Both halves are asserted because each alone admits a wrong close: a fast
/// return with no warning could be a join that happened to be quick, and the
/// warning with a slow return is the unbounded wait this contract forbids.
fn assert_close_abandons_within_budget(handler: &mut FemtoHTTPHandler) {
    let started = Instant::now();
    handler.close();
    let elapsed = started.elapsed();

    assert!(
        elapsed < CLOSE_ALLOWANCE,
        "close must not outlast its flush budget when the worker never \
         acknowledges: took {elapsed:?}, allowed {CLOSE_ALLOWANCE:?}, the \
         worker stays busy for {BUSY_WINDOW:?}"
    );
    let logged = crate::handlers::file::test_support::take_logged_messages();
    assert!(
        logged
            .iter()
            .any(|entry| entry.message.contains(ABANDON_MARKER)),
        "a close that did not wait for its worker must say it abandoned it; \
         close logged {logged:?}"
    );
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
#[serial_test::serial(http_close)]
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

    assert_close_abandons_within_budget(&mut handler);
}

/// The message `abandon_worker` logs, and the marker both close tests turn on.
const ABANDON_MARKER: &str = "abandoning it rather than blocking the caller";

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
#[serial_test::serial(http_close)]
fn a_close_whose_worker_answers_does_not_report_abandoning_it(
    tcp_listener: io::Result<TcpListener>,
) {
    let tcp_listener = tcp_listener.expect("bind ephemeral listener");
    let (addr, _rx) = spawn_mock_server(tcp_listener, 200).expect("spawn mock server");
    crate::handlers::file::test_support::install_test_logger();
    let mut handler = build_http_handler(addr);
    send_info_record(&handler, "healthy close").expect("record should be queued");

    handler.close();

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
