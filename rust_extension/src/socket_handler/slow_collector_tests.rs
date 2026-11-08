//! Contracts for the socket worker's write path against the shutdown budget.
//!
//! Split out of `drain_tests.rs`, which had grown past the 400-line limit this
//! repository sets. The split is a real one: `drain_tests.rs` is about what the
//! worker does with the shutdown *signal* - stopping a retry, discarding a
//! backlog - while this file is about a single frame that is already being
//! written when the deadline arrives.
//!
//! A socket write timeout bounds one `write` call, not one frame. A frame
//! larger than the sender's buffer therefore used to take as many timeouts as
//! it needed, none of which re-read the shutdown state, so the budget bounded
//! nothing the caller could see.

use std::io::{self, Read};
use std::net::{SocketAddr, TcpListener};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use rstest::rstest;

use super::drain::{ABANDON_MARKER, BUSY_WINDOW, CLOSE_ALLOWANCE, CLOSE_BUDGET, CONNECT_ALLOWANCE};
use super::{send_info_record, tcp_listener};
use crate::socket_handler::{
    FemtoSocketHandler, SocketHandlerConfig,
    config::BackoffPolicy,
    transport::{SocketTransport, TcpTransport},
};

/// How long the worker has to leave its write loop after `close` returns.
///
/// The shutdown budget plus the same 500ms margin [`CLOSE_ALLOWANCE`] carries
/// for scheduling. It has to be tied to the budget rather than set generously:
/// an allowance of seconds would pass a worker that ignored the deadline and
/// simply finished the frame, which is the defect this test exists to catch.
///
/// A collector that reads slowly enough for the frame to outlast this is what
/// makes that discrimination possible, so [`SLOW_READ_CHUNK`] and
/// [`SLOW_READ_PAUSE`] are chosen against this figure rather than in isolation.
const SLOW_COLLECTOR_ALLOWANCE: Duration = Duration::from_millis(700);
/// The bytes one read from the slow collector takes at a time.
///
/// One kilobyte makes the 3.9 MB message about 3,800 reads, so the pause below
/// dominates the transfer: the collector drains roughly a megabyte a second,
/// and finishing the frame takes about four seconds. That is far longer than
/// [`SLOW_COLLECTOR_ALLOWANCE`], so a worker that ignores the deadline cannot
/// reach the end of the frame inside it however fast the host schedules.
const SLOW_READ_CHUNK: usize = 1024;
/// How long the slow collector pauses between reads.
const SLOW_READ_PAUSE: Duration = Duration::from_millis(20);
/// The largest frame the slow-collector handler will accept.
const SLOW_MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
/// The message the write loop logs when it stops on the shutdown budget.
///
/// Taken from `frame_write`'s own error text, and deliberately the part that
/// names the cause rather than the progress figures: the value of the message
/// is that no socket timeout produces it, so a test that matches on it is
/// asserting the deadline was consulted and not merely that the frame ended.
const BUDGET_SPENT_MARKER: &str = "shutdown budget spent after writing";
/// A message far larger than the socket send buffer, so the write is partial.
///
/// Loopback accepted about 2.5 MiB before blocking when this was measured, and
/// this message leaves that figure 1.4 MiB behind, so no scheduling accident
/// can make the write complete inside the budget. It stays under the frame
/// limit because a rejected frame would never reach the socket at all.
const SLOW_MESSAGE_BYTES: usize = 3_900_000;

/// Accept one connection and read it slowly enough to keep the sender blocked.
///
/// Reports when the first byte arrives, so a test can wait for the worker to be
/// inside its write rather than sleeping for a guess. Every read is followed by
/// a pause, so the sender's buffer refills far slower than the frame writes it.
///
/// The connection is held until the reader is dropped at the end of the test,
/// which is what keeps a worker that ignores the budget from ever finishing.
fn spawn_slow_collector(listener: TcpListener) -> io::Result<(SocketAddr, mpsc::Receiver<()>)> {
    let addr = listener.local_addr()?;
    let (first_byte_tx, first_byte_rx) = mpsc::channel();
    thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut sink = vec![0_u8; SLOW_READ_CHUNK];
        let mut announced = false;
        while let Ok(read) = stream.read(&mut sink) {
            if read == 0 {
                return;
            }
            if !announced {
                announced = first_byte_tx.send(()).is_ok();
            }
            thread::sleep(SLOW_READ_PAUSE);
        }
    });
    Ok((addr, first_byte_rx))
}

/// Build a handler aimed at the slow collector with a large frame limit.
fn build_slow_collector_handler(addr: SocketAddr) -> FemtoSocketHandler {
    let config = SocketHandlerConfig {
        capacity: 8,
        transport: SocketTransport::Tcp(TcpTransport {
            host: addr.ip().to_string(),
            port: addr.port(),
            tls: None,
        }),
        connect_timeout: CLOSE_BUDGET,
        write_timeout: CLOSE_BUDGET,
        max_frame_size: SLOW_MAX_FRAME_BYTES,
        backoff: BackoffPolicy {
            base: Duration::from_secs(5),
            cap: Duration::from_secs(5),
            reset_after: Duration::from_secs(60),
            deadline: BUSY_WINDOW,
        },
        ..Default::default()
    };
    FemtoSocketHandler::with_config(config)
}

/// Scenario: a frame far larger than the socket buffer goes to a collector
/// that reads a little and then pauses.
///
/// Invariant: the worker stops inside the shutdown budget and exits, rather
/// than writing as many socket-timeout-sized chunks as the frame needs. The
/// write used to be a single `write_all`, and a socket timeout bounds each
/// call, not the frame: one 3.9 MB frame against a 1 KiB/20 ms reader takes
/// several seconds of successive timeouts, none of which re-read the shutdown
/// state or the remaining budget.
///
/// The assertion is the worker's exit, not the `close()` duration, because
/// `close` detaches a worker that misses its acknowledgement and returns
/// inside its budget either way. Only a worker that actually stopped proves
/// the write path consulted the deadline between calls.
///
/// Either close path is accepted, because both are correct and which one
/// happens is a race: the write ends at the same deadline the handler stops
/// waiting at, so an acknowledgement landing microseconds before it is joined
/// rather than abandoned. What the warning is held to is the path taken - a
/// close that gave up says so, and one that joined its worker does not - and
/// only the abandoned path is required to have stopped on the budget.
#[rstest]
#[serial_test::serial]
fn a_slow_collector_does_not_outlast_the_shutdown_budget(
    tcp_listener: std::io::Result<TcpListener>,
) {
    let listener = tcp_listener.expect("bind a slow collector");
    let (addr, first_byte) = spawn_slow_collector(listener).expect("spawn slow collector");
    crate::handlers::file::test_support::install_test_logger();
    let mut handler = build_slow_collector_handler(addr);
    send_info_record(&mut handler, &"x".repeat(SLOW_MESSAGE_BYTES)).expect("record should queue");
    first_byte
        .recv_timeout(CONNECT_ALLOWANCE)
        .expect("the worker should begin writing to the slow collector");

    let started = Instant::now();
    handler.close();
    let elapsed = started.elapsed();
    // Asked for after the close, because it reports on a worker `close` gave
    // up on and there is no such worker before that. A `close` that joined its
    // worker instead leaves nothing here, and the empty receiver below is read
    // as that outcome rather than as a worker that outlived the allowance.
    let worker = handler.worker_for_test();
    assert!(
        elapsed < CLOSE_ALLOWANCE,
        "close must not wait for a frame the collector is still reading: took \
         {elapsed:?}, allowed {CLOSE_ALLOWANCE:?}"
    );

    // `close` returns at the same deadline the write stops at, so the
    // acknowledgement that decides which path it took arrives within
    // microseconds of the decision. A `close` that detached its worker proves
    // the exit through this receiver; one that joined it has already proved it
    // by the join returning, and hands back a disconnected receiver because
    // there is no handle left to report on.
    let outcome = worker.recv_timeout(SLOW_COLLECTOR_ALLOWANCE);
    let was_abandoned = matches!(outcome, Ok(()));
    assert!(
        !matches!(outcome, Err(mpsc::RecvTimeoutError::Timeout)),
        "the worker must leave its write loop at the shutdown deadline rather \
         than finish the frame; it was still running {SLOW_COLLECTOR_ALLOWANCE:?} \
         after close returned in {elapsed:?}"
    );
    let logged = crate::handlers::file::test_support::take_logged_messages();
    let said_abandoned = logged
        .iter()
        .any(|entry| entry.message.contains(ABANDON_MARKER));
    assert_eq!(
        said_abandoned,
        was_abandoned,
        "close must report abandoning a worker only when it did; it {} its \
         worker, and {} that warning; it returned in {elapsed:?}, and close \
         logged {logged:?}",
        if was_abandoned { "abandoned" } else { "joined" },
        if said_abandoned {
            "emitted"
        } else {
            "did not emit"
        },
    );
    // What stopped the write is asserted only when `close` gave up on it. A
    // joined worker may have reached the end of the frame legitimately, and
    // demanding the budget message there would fail correct behaviour - that
    // is the mistaken assumption this test was originally written with.
    //
    // On the abandonment path the exit above is not enough on its own: a
    // `write` call that times out on its own per-call budget also ends the
    // frame, so a connection that broke and a deadline that was honoured would
    // both pass it. Only the write loop produces this message, and it produces
    // it by comparing the frame's remaining budget before starting another
    // call - which is exactly the consultation this test exists to require.
    if was_abandoned {
        assert!(
            logged
                .iter()
                .any(|entry| entry.message.contains(BUDGET_SPENT_MARKER)),
            "a worker left running at the deadline must stop because it re-read \
             the shutdown budget between write calls, not because one socket \
             timeout expired; a worker that kept writing until the frame \
             finished would also have exited; close logged {logged:?}"
        );
    }
}
