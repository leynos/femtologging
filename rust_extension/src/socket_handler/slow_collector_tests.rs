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
/// Whatever it is blocked on at that point - one socket timeout of at most
/// `CLOSE_BUDGET`, plus the last chunk it had already queued - is far shorter.
const SLOW_COLLECTOR_ALLOWANCE: Duration = Duration::from_secs(5);
/// The bytes one read from the slow collector takes at a time.
const SLOW_READ_CHUNK: usize = 4 * 1024;
/// How long the slow collector pauses between reads.
const SLOW_READ_PAUSE: Duration = Duration::from_millis(20);
/// The largest frame the slow-collector handler will accept.
const SLOW_MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
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
/// call, not the frame: one 4 MiB frame against a 4 KiB/20 ms reader took
/// several seconds of successive timeouts, none of which re-read the shutdown
/// state or the remaining budget.
///
/// The assertion is the worker's exit, not the `close()` duration, because
/// `close` detaches a worker that misses its acknowledgement and returns
/// inside its budget either way. Only a worker that actually stopped proves
/// the write path consulted the deadline between calls. The abandonment
/// warning is asserted too: a worker that exited because the connection broke
/// would also pass a bare exit check.
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
    // Asked for after the close: only the abandonment path keeps the handle,
    // so before this point there is no worker to report on.
    let worker = handler.worker_for_test();
    assert!(
        elapsed < CLOSE_ALLOWANCE,
        "close must not wait for a frame the collector is still reading: took \
         {elapsed:?}, allowed {CLOSE_ALLOWANCE:?}"
    );

    let joined = worker.recv_timeout(SLOW_COLLECTOR_ALLOWANCE);
    assert!(
        joined.is_ok(),
        "the worker must leave its write loop at the shutdown deadline rather \
         than finish the frame; it was still running {SLOW_COLLECTOR_ALLOWANCE:?} \
         after close returned in {elapsed:?}"
    );
    let logged = crate::handlers::file::test_support::take_logged_messages();
    assert!(
        logged
            .iter()
            .any(|entry| entry.message.contains(ABANDON_MARKER)),
        "a worker that missed its acknowledgement must say so; the exit above is \
         only evidence the write stopped, and a broken connection would explain \
         it too; close logged {logged:?}"
    );
}
