//! Contracts for `FemtoSocketHandler`'s close path.
//!
//! Split out of `tests.rs`, the way `http_handler/close_tests.rs` is split out
//! of its own, because the socket handler's close now has a budget to keep and
//! the fixtures that make a worker miss it do not belong among the framing
//! tests.
//!
//! The pair here is the same two halves the HTTP handler's close is held to.
//! One asserts that `close` returns inside its budget, and says it abandoned
//! the worker, when the worker never acknowledges; the other that a worker
//! which does acknowledge is joined rather than abandoned. The second exists
//! because the first does not need it: abandoning unconditionally passes every
//! other test in this crate.
//!
//! What the *worker* does with the shutdown signal, as opposed to what the
//! handler does with the acknowledgement, is asserted in `drain_tests.rs`.

use std::io::{self, Read};
use std::net::{SocketAddr, TcpListener};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use rstest::rstest;

use super::{HandlerError, send_info_record, spawn_stalled_tls_peer, tcp_listener};
use crate::handlers::{HandlerBuilderTrait, socket_builder::SocketHandlerBuilder};
use crate::socket_handler::{
    FemtoSocketHandler, SocketHandlerConfig,
    transport::{SocketTransport, TcpTransport, TlsOptions},
};

/// How long a test waits for the client hello to reach the stalled TLS peer.
const CONNECT_ALLOWANCE: Duration = Duration::from_secs(10);

/// How long the worker is kept busy, and so how long an unbounded join waits.
const BUSY_WINDOW: Duration = Duration::from_secs(10);
/// The handler's flush and shutdown budget.
const CLOSE_BUDGET: Duration = Duration::from_millis(200);
/// What `close` is allowed to take. Generous against `CLOSE_BUDGET` so a
/// loaded host cannot fail it, and far below `BUSY_WINDOW` so an unbounded
/// join cannot pass it.
const CLOSE_ALLOWANCE: Duration = Duration::from_millis(700);

/// The message `abandon_worker` logs, and the marker both close tests turn on.
const ABANDON_MARKER: &str = "abandoning it rather than blocking the caller";

/// Build a handler whose worker blocks in a TLS handshake with a stalled peer.
fn build_stuck_handler(addr: SocketAddr, capacity: usize) -> FemtoSocketHandler {
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
        // Longer than the handler's close budget, so the in-flight handshake
        // cannot finish before close gives up waiting for its acknowledgement.
        connect_timeout: BUSY_WINDOW,
        write_timeout: CLOSE_BUDGET,
        ..Default::default()
    };
    FemtoSocketHandler::with_config(config)
}

/// Wait until the worker has sent its TLS hello and is waiting for a reply.
fn await_tls_handshake(hello_sent: &mpsc::Receiver<()>) {
    hello_sent
        .recv_timeout(CONNECT_ALLOWANCE)
        .expect("the worker should reach the stalled TLS handshake");
}

/// Scenario: the worker is stuck in a TLS handshake with a peer that never
/// answers, so it cannot acknowledge the shutdown.
///
/// Invariant: `close` returns within its own flush budget rather than waiting
/// for the worker. Before this change the shutdown command was sent with a
/// blocking `send`, the acknowledgement's timeout was discarded with `let _ =`,
/// and the join that followed was unconditional - so a worker that never
/// answered held the caller until its in-flight network operation finished,
/// a figure nothing in the configuration describes. `close` runs from `Drop`, so a
/// logger being torn down could hold a thread for that whole window.
///
/// `CLOSE_BUDGET` is 200ms and `BUSY_WINDOW` is ten seconds. The assertion
/// allows the budget plus 500ms for scheduling, which is far short of the
/// window an unbounded join would wait out.
///
/// Proved by reverting: with the bound removed, this fails with an elapsed
/// time around the ten-second window. The abandonment warning is asserted too,
/// since a quick return alone could be a join that happened to be fast.
///
/// `#[serial]` because the tests here read one process-wide log capture: the
/// stuck closes assert the abandonment and discard messages are present, and
/// the healthy close asserts the first is absent. The key is the default one
/// rather than a name of this module's own, because `test_support`'s capture is
/// also read by the file handler's tests: a private key would let those drain
/// the buffer mid-assertion.
#[rstest]
#[serial_test::serial]
fn close_returns_within_its_budget_when_the_worker_never_acknowledges(
    tcp_listener: io::Result<TcpListener>,
) {
    let tcp_listener = tcp_listener.expect("bind ephemeral listener");
    let (addr, hello_sent, release_peer) =
        spawn_stalled_tls_peer(tcp_listener).expect("spawn stalled TLS peer");
    crate::handlers::file::test_support::install_test_logger();
    let mut handler = build_stuck_handler(addr, 16);
    send_info_record(&mut handler, "stuck in TLS handshake").expect("record should be queued");
    await_tls_handshake(&hello_sent);

    assert_close_abandons_within_budget(&mut handler);
    release_peer
        .send(())
        .expect("release the stalled TLS peer after close");
    let worker_logs = crate::handlers::file::test_support::wait_for_message(
        "FemtoSocketHandler dropped 1 records during shutdown",
        Duration::from_secs(2),
    );
    assert!(
        worker_logs.iter().any(|entry| entry
            .message
            .contains("FemtoSocketHandler dropped 1 records during shutdown")),
        "the stalled handshake should report its in-flight record as a shutdown drop; got {worker_logs:?}"
    );
}

/// Scenario: the worker is stuck in TLS and its command queue is full at close.
///
/// Invariant: `close` still returns within its budget. The shutdown command has
/// to be queued before any acknowledgement can be awaited, and on a full
/// bounded channel a plain send blocks until the worker next reads, which a
/// stuck worker does not do while the handshake is in flight.
///
/// A queue of one entry makes the state reachable deterministically: the worker
/// holds the first record in its handshake, and the second fills the queue.
///
/// Mutation proof: sending the shutdown command with a plain `send` fails this
/// test at roughly the ten-second connection timeout, and no other.
#[rstest]
#[serial_test::serial]
fn close_returns_within_its_budget_when_the_queue_is_full(tcp_listener: io::Result<TcpListener>) {
    let tcp_listener = tcp_listener.expect("bind ephemeral listener");
    let (addr, hello_sent, release_peer) =
        spawn_stalled_tls_peer(tcp_listener).expect("spawn stalled TLS peer");
    crate::handlers::file::test_support::install_test_logger();
    let mut handler = build_stuck_handler(addr, 1);
    send_info_record(&mut handler, "held in TLS handshake").expect("first record should queue");
    await_tls_handshake(&hello_sent);
    send_info_record(&mut handler, "fills the queue").expect("second record should queue");
    assert!(
        matches!(
            send_info_record(&mut handler, "overflows"),
            Err(HandlerError::QueueFull)
        ),
        "a further record should report QueueFull while TLS holds the worker"
    );

    assert_close_abandons_within_budget(&mut handler);
    release_peer
        .send(())
        .expect("release the stalled TLS peer after close");
    let worker_logs = crate::handlers::file::test_support::wait_for_message(
        "queued records at its shutdown deadline",
        Duration::from_secs(2),
    );
    assert!(
        worker_logs
            .iter()
            .any(|entry| entry.message.contains("discarded 1 queued records")),
        "the detached worker should report its queued record; got {worker_logs:?}"
    );
}

/// Close `handler`, asserting it returned inside the allowance and said it
/// abandoned the worker.
///
/// Both halves are asserted because each alone admits a wrong close: a fast
/// return with no warning could be a join that happened to be quick, and the
/// warning with a slow return is the unbounded wait this contract forbids.
fn assert_close_abandons_within_budget(handler: &mut FemtoSocketHandler) {
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

/// Scenario: the worker answers the shutdown request promptly.
///
/// Invariant: `close` joins it, and says nothing about abandoning anything.
///
/// This is the narrow half, and it was written because the sufficient half did
/// not need it. Replacing the whole decision with an unconditional
/// `abandon_worker` passes every other test in this crate. A bound that is
/// never taken is not a bound, and nothing would have failed.
///
/// The one thing the two paths do differ in is what they say, so the assertion
/// is on the message: a healthy close must not claim to have abandoned its
/// worker.
///
/// Mutation proof: replacing the match in `close` with an unconditional
/// `abandon_worker` fails this test on the warning it then emits.
#[rstest]
#[serial_test::serial]
fn a_close_whose_worker_answers_does_not_report_abandoning_it(
    tcp_listener: io::Result<TcpListener>,
) {
    let tcp_listener = tcp_listener.expect("bind ephemeral listener");
    let addr = tcp_listener.local_addr().expect("listener has address");
    thread::spawn(move || {
        let mut held = Vec::new();
        for stream in tcp_listener.incoming() {
            let Ok(mut stream) = stream else { break };
            // Drain what the client sends so its write completes and the
            // worker settles back into reading its command channel.
            let mut sink = vec![0_u8; 1 << 16];
            while let Ok(read) = stream.read(&mut sink) {
                if read == 0 {
                    break;
                }
            }
            held.push(stream);
        }
    });
    crate::handlers::file::test_support::install_test_logger();
    let mut handler = SocketHandlerBuilder::new()
        .with_tcp(addr.ip().to_string(), addr.port())
        // Leave an idle worker enough scheduling room to acknowledge shutdown
        // on a loaded host; other close tests exercise the one-second bound.
        .with_write_timeout_ms(5_000)
        .build_inner()
        .expect("build handler");
    send_info_record(&mut handler, "healthy close").expect("record should be queued");
    // Let the worker connect and send before the close, so the close is
    // answered by a worker that is idle rather than one still connecting.
    await_idle(&handler);

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

/// Wait until the worker has sent everything queued ahead of this point.
///
/// The worker processes commands in order and acknowledges a flush after it has
/// written what came before it, so a successful flush is evidence that the
/// record is gone rather than a sleep long enough to hope so. That is what the
/// healthy case needs: a close answered by a worker that is idle and reading
/// its channel, not one that is still mid-write.
fn await_idle(handler: &FemtoSocketHandler) {
    let delivered = handler.flush();
    assert!(delivered, "the worker should acknowledge a flush");
}
