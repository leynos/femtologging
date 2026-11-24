//! Tests for socket-worker shutdown command ordering.
//!
//! `queued_shutdown_ack_waits_for_records_after_it` checks that a
//! `SocketCommand::Shutdown` acknowledgement follows the records queued
//! behind it, using `Worker::drain_pending` against a stalled TLS handshake.
//! The expired-budget cases show that a connect or write whose
//! `ShutdownSignal` budget has already run out drops the current record rather
//! than starting work. `shutdown_drop_is_logged_inside_the_warning_interval`
//! covers the warning path in `Worker::warn_shutdown_drops`, and
//! `a_flush_reports_the_workers_suppressed_drop_before_acknowledging` pins the
//! worker-side count a normal flush reports before acknowledging.

use super::*;
use std::{
    io::Read,
    net::TcpListener,
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

use crate::{
    handlers::file::test_support::{install_test_logger, take_logged_messages, wait_for_message},
    level::FemtoLevel,
    log_record::FemtoLogRecord,
    rate_limited_warner::Clock,
    shutdown_signal::ShutdownSignal,
    socket_handler::transport::{SocketTransport, TcpTransport, TlsOptions},
};

/// A clock that never advances, so the warning interval cannot elapse.
struct FixedClock;

impl Clock for FixedClock {
    fn now_millis(&self) -> u64 {
        0
    }
}

/// The warning a record abandoned by the shutdown budget emits.
///
/// The worker counts into the same cumulative warner as the handler, and a
/// refusal at the shutdown deadline is the last chance to report its count, so
/// the wording names the cause that triggered the flush.
const SHUTDOWN_DROP: &str = concat!(
    "FemtoSocketHandler dropped 1 records since the last report; ",
    "triggering cause: abandoned retry during shutdown"
);

#[test]
#[serial_test::serial]
fn queued_shutdown_ack_waits_for_records_after_it() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind TLS test listener");
    let address = listener.local_addr().expect("listener has address");
    let (hello_tx, hello_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept TLS connection");
        let mut record_header = [0_u8; 5];
        stream
            .read_exact(&mut record_header)
            .expect("read TLS client hello");
        hello_tx.send(()).expect("notify that handshake started");
        release_rx.recv().expect("wait for test release");
    });

    let signal = Arc::new(ShutdownSignal::new());
    let config = SocketHandlerConfig {
        transport: SocketTransport::Tcp(TcpTransport {
            host: address.ip().to_string(),
            port: address.port(),
            tls: Some(TlsOptions {
                domain: "localhost".into(),
                insecure_skip_verify: true,
            }),
        }),
        connect_timeout: Duration::from_secs(2),
        write_timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let mut worker = Worker::new(config, Arc::clone(&signal));
    let (command_tx, command_rx) = bounded(2);
    let (shutdown_tx, shutdown_rx) = bounded(1);
    command_tx
        .send(SocketCommand::Shutdown(shutdown_tx))
        .expect("queue shutdown");
    command_tx
        .send(SocketCommand::Record(FemtoLogRecord::new(
            "test",
            FemtoLevel::Info,
            "after shutdown",
        )))
        .expect("queue record behind shutdown");
    signal.begin_shutdown(Instant::now() + Duration::from_secs(2));

    let drain = thread::spawn(move || worker.drain_pending(&command_rx, None));
    hello_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("record after shutdown should reach the TLS handshake");
    let acknowledgement_was_pending = matches!(
        shutdown_rx.try_recv(),
        Err(crossbeam_channel::TryRecvError::Empty)
    );

    release_tx
        .send(())
        .expect("release the stalled TLS handshake");
    if acknowledgement_was_pending {
        shutdown_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("shutdown acknowledgement should follow the record");
    }
    drain.join().expect("worker drain should return");
    server.join().expect("TLS server should return");
    assert!(
        acknowledgement_was_pending,
        "shutdown acknowledgement must wait until later queued records finish"
    );
}

#[test]
#[serial_test::serial]
fn expired_connect_budget_counts_the_current_record_as_a_shutdown_drop() {
    crate::handlers::file::test_support::install_test_logger();
    let signal = Arc::new(ShutdownSignal::new());
    signal.begin_shutdown(Instant::now());
    let mut worker = Worker::new(SocketHandlerConfig::default(), signal);

    worker.handle_record_command(FemtoLogRecord::new(
        "test",
        FemtoLevel::Info,
        "budget expired before connect",
    ));

    let messages = crate::handlers::file::test_support::take_logged_messages();
    assert!(
        messages
            .iter()
            .any(|entry| entry.message.contains(SHUTDOWN_DROP))
    );
}

#[test]
#[serial_test::serial]
fn expired_write_budget_counts_the_current_record_as_a_shutdown_drop() {
    crate::handlers::file::test_support::install_test_logger();
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind socket test listener");
    let address = listener.local_addr().expect("listener has address");
    let stream = std::net::TcpStream::connect(address).expect("connect to test listener");
    let _peer = listener.accept().expect("accept test connection").0;
    let signal = Arc::new(ShutdownSignal::new());
    signal.begin_shutdown(Instant::now());
    let mut worker = Worker::new(SocketHandlerConfig::default(), signal);
    worker.connection = Some(ActiveConnection::PlainTcp(stream));

    worker.send_frame_to_connection(b"frame", Instant::now());

    let messages = crate::handlers::file::test_support::take_logged_messages();
    assert!(
        messages
            .iter()
            .any(|entry| entry.message.contains(SHUTDOWN_DROP))
    );
}

/// The worker's own suppressed count is reported before it acknowledges.
///
/// The handler and the worker keep separate warners, and the flush marker
/// carries only the handler-side count, so a worker-side drop that the interval
/// suppresses reaches the caller only if the worker reports its own warner when
/// it handles the marker.
///
/// The acknowledgement is sent on a rendezvous channel, so its send cannot
/// complete until this test receives. A worker that acknowledged before
/// reporting would therefore sit in the send with no report in the buffer, and
/// the wait below would time out; only a report made first lets it be found.
#[test]
#[serial_test::serial]
fn a_flush_reports_the_workers_suppressed_drop_before_acknowledging() {
    install_test_logger();
    let mut worker = Worker::new(
        SocketHandlerConfig::default(),
        Arc::new(ShutdownSignal::new()),
    );
    worker.warner = RateLimitedWarner::with_clock(Duration::from_secs(5), Arc::new(FixedClock));
    // Two drops the handler never sees: the first warns, the second is
    // suppressed and stays pending, which a flush has to report. Recorded
    // through the production helper so the count and the interval behave as
    // they do for a real worker-side failure.
    warn_drops_naming_cause(&worker.warner, "serialization failure");
    warn_drops_naming_cause(&worker.warner, "serialization failure");
    let _ = take_logged_messages();

    let (ack_tx, ack_rx) = bounded(0);
    let flush = thread::spawn(move || worker.handle_flush_command(ack_tx, None));

    let reported = wait_for_message(
        "FemtoSocketHandler dropped 1 records since the last report",
        Duration::from_secs(2),
    );
    assert!(
        reported
            .iter()
            .any(|entry| entry.message
                == "FemtoSocketHandler dropped 1 records since the last report"),
        "the worker's suppressed drop must be reported before the acknowledgement: {reported:?}"
    );
    ack_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("the flush must still acknowledge")
        .expect("a flush with no pending writes reports success");
    flush.join().expect("the worker flush should return");
}

#[test]
#[serial_test::serial]
fn shutdown_drop_is_logged_inside_the_warning_interval() {
    crate::handlers::file::test_support::install_test_logger();
    let worker = Worker::new(
        SocketHandlerConfig::default(),
        Arc::new(ShutdownSignal::new()),
    );
    worker.warner.record_drop();
    worker.warner.warn_if_due(|_| {});

    worker.warn_shutdown_drops();

    let messages = crate::handlers::file::test_support::take_logged_messages();
    assert!(
        messages
            .iter()
            .any(|entry| entry.message.contains(SHUTDOWN_DROP))
    );
}
