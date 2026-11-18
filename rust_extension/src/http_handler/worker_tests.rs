//! Unit tests for the HTTP worker's own helpers.
//!
//! Response classification has its own file, `response_classification_tests.rs`,
//! and request decoration has `request_auth_tests.rs`; neither is repeated
//! here. What is left is the worker's shutdown and queue behaviour.

use super::*;
use std::{
    io::Read,
    net::TcpListener,
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

use rstest::rstest;

use crate::{level::FemtoLevel, shutdown_signal::ShutdownSignal};

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

/// A flush command reports worker drops before it acknowledges.
///
/// The worker's warner is shared across causes, so a drop arriving inside the
/// interval is only counted, not reported. Without this flush the count would
/// wait for the next elapse - or never be seen, once the worker is gone. The
/// handler treats the acknowledgement as "reporting finished", so the warning
/// must be emitted before it: the acknowledgement channel here has no buffer,
/// so its send cannot complete until this test receives, and capturing the
/// warning before that receive proves the order rather than assuming it.
#[test]
#[serial_test::serial]
fn the_flush_command_reports_drops_before_acknowledging() {
    crate::handlers::file::test_support::install_test_logger();
    let mut worker = Worker::new(
        HTTPHandlerConfig::default(),
        Arc::new(ShutdownSignal::new()),
    );
    // Open the interval with a reported drop, then record one that the
    // interval suppresses.
    worker.warner.record_drop();
    worker.warner.warn_if_due(|_| {});
    worker.warner.record_drop();
    assert_eq!(worker.warner.pending_drops(), 1);

    let (ack_tx, ack_rx) = bounded(0);
    thread::scope(|scope| {
        let worker = &mut worker;
        scope.spawn(move || worker.handle_flush_command(ack_tx));
        let messages = crate::handlers::file::test_support::wait_for_message(
            "dropped 1 records since the last report",
            Duration::from_secs(5),
        );
        // Release the worker before asserting: an assertion that fails while
        // the sender is still blocked in a rendezvous send would leave
        // `thread::scope` joining a thread this test can no longer release.
        let acknowledged = ack_rx.recv_timeout(Duration::from_secs(5)).is_ok();
        assert!(acknowledged, "the flush command must be acknowledged");
        assert!(
            messages
                .iter()
                .any(|entry| entry.message.contains("dropped 1 records")),
            "the flush must report the held-back count before acknowledging: {messages:?}"
        );
    });
    assert_eq!(
        worker.warner.pending_drops(),
        0,
        "the flush must swap the count out, not merely read it"
    );
}

/// Which command a drain is left to acknowledge.
#[derive(Clone, Copy)]
enum PendingAck {
    /// The normal path: a shutdown command queued before the deadline.
    Shutdown,
    /// The aborted path: the deadline has passed with a flush still queued.
    FlushAfterAbort,
}

/// A drain reports the drops the interval held back before it acknowledges.
///
/// Scenario: a worker-side drop is recorded inside the warning interval, then
/// the worker drains a queue holding one command whose acknowledgement the
/// caller waits on. Invariant: the count is reported before that
/// acknowledgement, and nothing is left pending, on the normal path and on the
/// aborted path alike. The aborted path is the one that can regress:
/// `discard_rest` acknowledges a queued flush itself, so a report emitted
/// after it reaches a caller that has already returned.
///
/// The acknowledgement channel is a rendezvous, so capturing the warning before
/// the test receives proves the order rather than assuming it.
#[rstest]
#[case::shutdown(PendingAck::Shutdown)]
#[case::flush_after_abort(PendingAck::FlushAfterAbort)]
#[serial_test::serial]
fn a_drain_reports_drops_before_acknowledging(#[case] pending: PendingAck) {
    use crate::handlers::file::test_support::{install_test_logger, wait_for_message};

    install_test_logger();
    let signal = Arc::new(ShutdownSignal::new());
    if matches!(pending, PendingAck::FlushAfterAbort) {
        signal.abort();
    }
    let mut worker = Worker::new(HTTPHandlerConfig::default(), Arc::clone(&signal));
    // Open the interval with a reported drop, then record one it suppresses.
    worker.warner.record_drop();
    worker.warner.warn_if_due(|_| {});
    worker.warner.record_drop();
    assert_eq!(worker.warner.pending_drops(), 1);

    let (command_tx, command_rx) = bounded(1);
    let (ack_tx, ack_rx) = bounded(0);
    let command = match pending {
        PendingAck::Shutdown => HTTPCommand::Shutdown(ack_tx),
        PendingAck::FlushAfterAbort => HTTPCommand::Flush(ack_tx),
    };
    command_tx.send(command).expect("queue the drain command");

    thread::scope(|scope| {
        let worker = &mut worker;
        scope.spawn(move || worker.drain_pending(&command_rx, None));
        let messages = wait_for_message(
            "dropped 1 records since the last report",
            Duration::from_secs(5),
        );
        // Release the worker before asserting: an assertion that fails while
        // the worker is blocked in a rendezvous acknowledgement send would
        // leave `thread::scope` joining a thread this test can no longer
        // release.
        let acknowledged = ack_rx.recv_timeout(Duration::from_secs(5)).is_ok();
        assert!(
            acknowledged,
            "the queued drain command must be acknowledged"
        );
        assert!(
            messages
                .iter()
                .any(|entry| entry.message.contains("dropped 1 records")),
            "the drain must report the held-back count before acknowledging: \
             {messages:?}"
        );
    });
    assert_eq!(
        worker.warner.pending_drops(),
        0,
        "the drain must leave nothing pending"
    );
}
