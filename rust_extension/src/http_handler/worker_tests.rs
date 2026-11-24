//! Unit tests for the HTTP worker's shutdown and drop accounting.
//!
//! Response classification has its own file, `response_classification_tests.rs`,
//! and request construction has `transport_tests.rs`; neither is repeated here.
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
    rate_limited_warner::Clock,
    shutdown_signal::ShutdownSignal,
};

/// A clock that never advances, so the warning interval cannot elapse.
struct FixedClock;

impl Clock for FixedClock {
    fn now_millis(&self) -> u64 {
        0
    }
}

#[test]
#[serial_test::serial]
fn queued_shutdown_ack_waits_for_records_after_it() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind HTTP test listener");
    let address = listener.local_addr().expect("listener has address");
    let (request_tx, request_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept HTTP request");
        let mut first_byte = [0_u8; 1];
        stream
            .read_exact(&mut first_byte)
            .expect("read request start");
        request_tx.send(()).expect("notify that request started");
        release_rx.recv().expect("wait for test release");
    });

    let signal = Arc::new(ShutdownSignal::new());
    let config = HTTPHandlerConfig {
        url: format!("http://{address}/log"),
        connect_timeout: Duration::from_secs(1),
        write_timeout: Duration::from_secs(2),
        ..Default::default()
    };
    let mut worker = Worker::new(config, Arc::clone(&signal));
    let (command_tx, command_rx) = bounded(2);
    let (shutdown_tx, shutdown_rx) = bounded(1);
    command_tx
        .send(HTTPCommand::Shutdown(shutdown_tx))
        .expect("queue shutdown");
    command_tx
        .send(HTTPCommand::Record(FemtoLogRecord::new(
            "test",
            FemtoLevel::Info,
            "after shutdown",
        )))
        .expect("queue record behind shutdown");
    signal.begin_shutdown(Instant::now() + Duration::from_secs(2));

    let drain = thread::spawn(move || worker.drain_pending(&command_rx, None));
    request_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("record after shutdown should reach the HTTP worker");
    let acknowledgement_was_pending = matches!(
        shutdown_rx.try_recv(),
        Err(crossbeam_channel::TryRecvError::Empty)
    );

    release_tx
        .send(())
        .expect("release the stalled HTTP request");
    if acknowledgement_was_pending {
        shutdown_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("shutdown acknowledgement should follow the record");
    }
    drain.join().expect("worker drain should return");
    server.join().expect("HTTP server should return");
    assert!(
        acknowledgement_was_pending,
        "shutdown acknowledgement must wait until later queued records finish"
    );
}

#[test]
#[serial_test::serial]
fn aborted_request_counts_the_current_record_as_a_shutdown_drop() {
    crate::handlers::file::test_support::install_test_logger();
    let signal = Arc::new(ShutdownSignal::new());
    signal.abort();
    let mut worker = Worker::new(HTTPHandlerConfig::default(), signal);

    worker.send_request("payload");

    let messages = crate::handlers::file::test_support::take_logged_messages();
    assert!(messages.iter().any(|entry| {
        entry
            .message
            .contains("FemtoHTTPHandler dropped 1 records since the last report; triggering cause: abandoned retry during shutdown")
    }));
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
        HTTPHandlerConfig::default(),
        Arc::new(ShutdownSignal::new()),
    );
    worker.warner = RateLimitedWarner::with_clock(Duration::from_secs(5), Arc::new(FixedClock));
    // Two delivery failures inside one interval: the first warns, the second is
    // suppressed and stays pending, which is the count a flush has to report.
    worker.warn_permanent_drops();
    worker.warn_permanent_drops();
    let _ = take_logged_messages();

    let (ack_tx, ack_rx) = bounded(0);
    let flush = thread::spawn(move || worker.handle_flush_command(ack_tx, None));

    let reported = wait_for_message(
        "FemtoHTTPHandler dropped 1 records since the last report",
        Duration::from_secs(2),
    );
    assert!(
        reported.iter().any(
            |entry| entry.message == "FemtoHTTPHandler dropped 1 records since the last report"
        ),
        "the worker's suppressed drop must be reported before the acknowledgement: {reported:?}"
    );
    ack_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("the flush must still acknowledge");
    flush.join().expect("the worker flush should return");
}

#[test]
#[serial_test::serial]
fn shutdown_drop_is_logged_inside_the_warning_interval() {
    crate::handlers::file::test_support::install_test_logger();
    let worker = Worker::new(
        HTTPHandlerConfig::default(),
        Arc::new(ShutdownSignal::new()),
    );
    worker.warner.record_drop();
    worker.warner.warn_if_due(|_| {});

    worker.warn_shutdown_drops();

    let messages = crate::handlers::file::test_support::take_logged_messages();
    assert!(messages.iter().any(|entry| {
        entry
            .message
            .contains("FemtoHTTPHandler dropped 1 records since the last report; triggering cause: abandoned retry during shutdown")
    }));
}
