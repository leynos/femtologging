//! Contracts for what the socket worker does with the shutdown signal.
//!
//! Split from `close_tests.rs` along the same axis `http_handler` splits its
//! own: that file asserts what the *handler* does with the acknowledgement,
//! this one what the *worker* does with the signal. The signal is the half
//! that reaches a worker which is not reading its command channel, so these
//! are the contracts a stuck worker is held to rather than an idle one.
//!
//! These tests need workers that cannot reach their collectors: one that is in
//! a backoff delay when the close lands, and one stalled in TLS with a backlog
//! still queued when the deadline passes. A reachable collector would have the
//! worker idle and answering, which is the case `close_tests.rs` covers.

use std::io;
use std::net::{SocketAddr, TcpListener};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use rstest::rstest;

use super::{send_info_record, spawn_stalled_tls_peer, tcp_listener};
use crate::socket_handler::{
    FemtoSocketHandler, SocketHandlerConfig,
    config::BackoffPolicy,
    transport::{SocketTransport, TcpTransport, TlsOptions},
};

/// How long a test waits for the worker to reach a state it asserts on.
const CONNECT_ALLOWANCE: Duration = Duration::from_secs(10);

/// How long the worker would keep retrying, and so how long an unbounded close
/// against one waits.
const BUSY_WINDOW: Duration = Duration::from_secs(10);
/// The handler's flush and shutdown budget.
const CLOSE_BUDGET: Duration = Duration::from_millis(200);
/// The shutdown budget plus 500ms for scheduling.
const CLOSE_ALLOWANCE: Duration = Duration::from_millis(700);

/// The message `abandon_worker` logs, and the marker a close that gave up on
/// its worker turns on.
const ABANDON_MARKER: &str = "abandoning it rather than blocking the caller";
/// The message `warn_discarded` logs, and the marker the backlog test turns on.
const DISCARD_MARKER: &str = "queued records at its shutdown deadline";

/// Reserve and release an address so a connect there is refused immediately.
fn closed_port() -> io::Result<SocketAddr> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let addr = listener.local_addr()?;
    drop(listener);
    Ok(addr)
}

/// Build a handler that repeatedly fails to connect and enters backoff.
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
            // Keep the worker in its first backoff long enough for the test to
            // request shutdown deterministically after the wait hook fires.
            base: Duration::from_secs(5),
            cap: Duration::from_secs(5),
            reset_after: Duration::from_secs(60),
            deadline: BUSY_WINDOW,
        },
        ..Default::default()
    };
    FemtoSocketHandler::with_config(config)
}

/// Build a handler whose worker blocks in TLS with a peer that never answers.
fn build_stalled_handler(addr: SocketAddr, capacity: usize) -> FemtoSocketHandler {
    let config = SocketHandlerConfig {
        capacity,
        transport: SocketTransport::Tcp(TcpTransport {
            host: addr.ip().to_string(),
            port: addr.port(),
            tls: Some(TlsOptions {
                domain: "localhost".into(),
                insecure_skip_verify: true,
            }),
        }),
        connect_timeout: BUSY_WINDOW,
        write_timeout: CLOSE_BUDGET,
        ..Default::default()
    };
    FemtoSocketHandler::with_config(config)
}

/// Scenario: every connect attempt is refused, and the worker sleeps between
/// attempts.
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
    let (backoffs, resume_backoff) = handler.pause_next_wait();
    send_info_record(&mut handler, "unreachable").expect("record should be queued");
    backoffs
        .recv_timeout(CONNECT_ALLOWANCE)
        .expect("the worker should enter a connect backoff");
    let next_backoff = handler.wait_notification();
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
            shutdown_wait_started.elapsed() < CONNECT_ALLOWANCE,
            "close should publish shutdown before the paused wait resumes"
        );
        thread::yield_now();
    }
    resume_backoff
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
        "close must not wait out a connect backoff: took {elapsed:?}, allowed \
         {CLOSE_ALLOWANCE:?}"
    );
    assert!(
        matches!(
            next_backoff.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ),
        "closing during backoff must stop this record from being retried"
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

/// Scenario: the worker stalls in TLS while records fill its queue, and the
/// shutdown deadline passes before it can process them.
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
fn a_queue_that_outlasts_the_budget_is_discarded_and_counted(
    tcp_listener: std::io::Result<TcpListener>,
) {
    let listener = tcp_listener.expect("bind a stalled TLS peer");
    let (addr, hello_sent, release_peer) =
        spawn_stalled_tls_peer(listener).expect("spawn stalled TLS peer");
    crate::handlers::file::test_support::install_test_logger();
    let mut handler = build_stalled_handler(addr, 32);
    send_info_record(&mut handler, "held in TLS handshake").expect("first record should queue");
    hello_sent
        .recv_timeout(CONNECT_ALLOWANCE)
        .expect("the worker should enter the stalled TLS handshake");
    for index in 0..15 {
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
    release_peer
        .send(())
        .expect("release the stalled TLS peer after close");
    let logged = crate::handlers::file::test_support::wait_for_message(
        DISCARD_MARKER,
        Duration::from_secs(2),
    );
    assert!(
        logged.iter().any(|entry| {
            entry.message.contains("discarded 15 queued records")
                && entry.message.contains(DISCARD_MARKER)
        }),
        "a shutdown that discards queued records must say how many; close \
         logged {logged:?}"
    );
}
