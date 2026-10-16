//! Contracts for what the socket worker does with the shutdown signal.
//!
//! Split from `close_tests.rs` along the same axis `http_handler` splits its
//! own: that file asserts what the *handler* does with the acknowledgement,
//! this one what the *worker* does with the signal. The signal is the half
//! that reaches a worker which is not reading its command channel, so these
//! are the contracts a stuck worker is held to rather than an idle one.
//!
//! Both tests here need a worker that cannot reach its collector: one that is
//! inside a backoff delay when the close lands, and one with a backlog still
//! queued when the deadline passes. A reachable collector would have the worker
//! idle and answering, which is the case `close_tests.rs` covers.

use std::io;
use std::net::{SocketAddr, TcpListener};
use std::time::{Duration, Instant};

use rstest::rstest;

use super::send_info_record;
use crate::socket_handler::{
    FemtoSocketHandler, SocketHandlerConfig,
    config::BackoffPolicy,
    transport::{SocketTransport, TcpTransport},
};

/// How long a test waits for the worker to reach a state it asserts on.
const CONNECT_ALLOWANCE: Duration = Duration::from_secs(10);

/// How long the worker would keep retrying, and so how long an unbounded close
/// against one waits.
const BUSY_WINDOW: Duration = Duration::from_secs(10);
/// The handler's flush and shutdown budget.
const CLOSE_BUDGET: Duration = Duration::from_millis(200);
/// What `close` is allowed to take. Generous against `CLOSE_BUDGET` so a
/// loaded host cannot fail it, and far below `BUSY_WINDOW` so an unbounded
/// join cannot pass it.
const CLOSE_ALLOWANCE: Duration = Duration::from_secs(3);

/// The message `abandon_worker` logs, and the marker a close that gave up on
/// its worker turns on.
const ABANDON_MARKER: &str = "abandoning it rather than blocking the caller";
/// The message `warn_discarded` logs, and the marker the backlog test turns on.
const DISCARD_MARKER: &str = "queued records at its shutdown deadline";

/// An address nothing is listening on.
///
/// Binding an ephemeral port and dropping the listener releases it, so a
/// connect there fails rather than hanging. The failure is what these contracts
/// need: a worker whose connect attempt never succeeds is a worker that is
/// backing off rather than reading its command channel.
fn closed_port() -> io::Result<SocketAddr> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let addr = listener.local_addr()?;
    drop(listener);
    Ok(addr)
}

/// Build a handler pointed at an address nothing is listening on.
///
/// The timeouts are short so the worker cycles through failed connects rather
/// than spending each attempt waiting for one, and the backoff never expires
/// within a test's patience: a worker here is retrying forever, which is what
/// makes "does the close interrupt it" the only question left.
fn build_unconnectable_handler(addr: SocketAddr) -> FemtoSocketHandler {
    let config = SocketHandlerConfig {
        capacity: 32,
        transport: SocketTransport::Tcp(TcpTransport {
            host: addr.ip().to_string(),
            port: addr.port(),
            tls: None,
        }),
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
    FemtoSocketHandler::with_config(config)
}

/// Scenario: every connect attempt fails, and the worker sleeps between them.
///
/// Invariant: a close lands during one of those sleeps and is answered there,
/// rather than after it. The retry delay used to be `thread::sleep`, which no
/// shutdown can interrupt, so a close against an unreachable collector waited
/// out a delay the backoff policy chose and the caller never asked for.
///
/// The worker is known to be inside the delay before the close is issued: the
/// notification fires when `wait` begins, so this asserts an interruption rather
/// than a delay short enough to guess. A `thread::sleep` would miss the signal
/// entirely, and the close would take the abandonment path - which is the other
/// thing asserted, because a worker that wakes should be joined.
///
/// `#[serial]` because the tests here read one process-wide log capture: the
/// stuck closes assert the abandonment and discard messages are present, and
/// the healthy close asserts the first is absent. The key is the default one
/// rather than a name of this module's own, because `test_support`'s capture is
/// also read by the file handler's tests: a private key would let those drain
/// the buffer mid-assertion.
#[rstest]
#[serial_test::serial]
fn close_during_a_connect_backoff_is_answered_from_it() {
    let addr = closed_port().expect("reserve an unlistened address");
    crate::handlers::file::test_support::install_test_logger();
    let mut handler = build_unconnectable_handler(addr);
    let backoffs = handler.wait_notification();
    send_info_record(&mut handler, "unreachable").expect("record should be queued");
    backoffs
        .recv_timeout(CONNECT_ALLOWANCE)
        .expect("the worker should enter a connect backoff");

    let started = Instant::now();
    handler.close();
    let elapsed = started.elapsed();

    assert!(
        elapsed < CLOSE_ALLOWANCE,
        "close must not wait out a connect backoff: took {elapsed:?}, allowed \
         {CLOSE_ALLOWANCE:?}"
    );
    let logged = crate::handlers::file::test_support::take_logged_messages();
    let abandoned: Vec<_> = logged
        .iter()
        .filter(|entry| entry.message.contains(ABANDON_MARKER))
        .collect();
    assert!(
        abandoned.is_empty(),
        "a worker interrupted during a backoff must acknowledge the shutdown \
         rather than be abandoned; close logged {abandoned:?}"
    );
}

/// Scenario: a queue of records waits behind a collector that cannot be
/// reached, and the shutdown deadline passes before they are written.
///
/// Invariant: the records still queued at the deadline are discarded rather
/// than written one attempt at a time, and the count is warned about. A drain
/// that processed the whole backlog would spend one connect timeout per record,
/// which is a close that runs for as long as the queue is deep.
///
/// The contract that shares this round trip - that a record delivered before
/// the deadline still arrives - is asserted by the healthy-close test in
/// `close_tests.rs`, which flushes a record through a live connection before
/// closing.
#[rstest]
#[serial_test::serial]
fn a_queue_that_outlasts_the_budget_is_discarded_and_counted() {
    let addr = closed_port().expect("reserve an unlistened address");
    crate::handlers::file::test_support::install_test_logger();
    let mut handler = build_unconnectable_handler(addr);
    // The worker is occupied by the first record's failing connect, so the rest
    // are still in the queue when the close arrives.
    for index in 0..16 {
        send_info_record(&mut handler, &format!("backlog {index}"))
            .expect("record should be queued");
    }

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
