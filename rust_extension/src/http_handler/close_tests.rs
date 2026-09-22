//! Contracts for `FemtoHTTPHandler`'s close path.
//!
//! Split out of `tests.rs`, which reached the 400-line module limit, and split
//! here rather than anywhere else because these two tests share a fixture
//! nothing else uses: a server that accepts a request and never answers it.
//!
//! The pair is the two halves of one rule. One asserts that `close` returns
//! inside its flush budget when the worker never acknowledges; the other that
//! a worker which does acknowledge is joined rather than abandoned. The second
//! exists because the first does not need it: abandoning unconditionally
//! passed every other test in the crate.
//!
//! Included with `#[path]` from `tests.rs`, the way
//! `response_classification_tests.rs` already is, so both files stay inside
//! the limit and the helpers above remain in scope.

use std::io::{self, Read};
use std::net::{SocketAddr, TcpListener};
use std::thread;
use std::time::Duration;

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
fn spawn_silent_server(listener: TcpListener) -> io::Result<SocketAddr> {
    let addr = listener.local_addr()?;
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
        }
    });
    Ok(addr)
}

/// How long the worker is kept busy, and so how long an unbounded join waits.
const BUSY_WINDOW: Duration = Duration::from_secs(10);
/// The handler's flush and shutdown budget.
const CLOSE_BUDGET: Duration = Duration::from_millis(200);
/// What `close` is allowed to take. Generous against `CLOSE_BUDGET` so a
/// loaded host cannot fail it, and far below `BUSY_WINDOW` so an unbounded
/// join cannot pass it.
const CLOSE_ALLOWANCE: Duration = Duration::from_secs(3);

/// Build a handler pointed at a server that never replies.
fn build_stuck_handler(addr: SocketAddr) -> FemtoHTTPHandler {
    use crate::socket_handler::BackoffPolicy;

    let config = HTTPHandlerConfig {
        url: format!("http://{addr}/log"),
        method: HTTPMethod::POST,
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
/// time around the ten-second window.
///
/// `#[serial]` because its sibling below reads a process-wide log capture and
/// this test is the only thing in the binary that emits the message that
/// sibling asserts the absence of.
#[rstest]
#[serial_test::serial(http_close)]
fn close_returns_within_its_budget_when_the_worker_never_acknowledges(
    tcp_listener: io::Result<TcpListener>,
) {
    let tcp_listener = tcp_listener.expect("bind ephemeral listener");
    let addr = spawn_silent_server(tcp_listener).expect("spawn silent server");
    let mut handler = build_stuck_handler(addr);
    send_info_record(&handler, "stuck").expect("record should be queued");

    // Let the worker take the record and enter its request before closing, so
    // the shutdown command arrives at a worker that cannot read it. Without
    // this the worker might acknowledge before it ever sends, and the test
    // would pass without exercising the path it names.
    thread::sleep(CLOSE_BUDGET);

    let started = std::time::Instant::now();
    handler.close();
    let elapsed = started.elapsed();

    assert!(
        elapsed < CLOSE_ALLOWANCE,
        "close must not outlast its flush budget when the worker never \
         acknowledges: took {elapsed:?}, allowed {CLOSE_ALLOWANCE:?}, the \
         worker stays busy for {BUSY_WINDOW:?}"
    );
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
