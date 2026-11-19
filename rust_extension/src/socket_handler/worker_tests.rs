//! Tests for socket-worker shutdown command ordering.

use super::*;
use std::{
    io::Read,
    net::TcpListener,
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

use rstest::rstest;

use crate::{
    level::FemtoLevel,
    log_record::FemtoLogRecord,
    shutdown_signal::ShutdownSignal,
    socket_handler::transport::{SocketTransport, TcpTransport, TlsOptions},
};

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
    assert!(messages.iter().any(|entry| {
        entry
            .message
            .contains("FemtoSocketHandler dropped 1 records since the last report; triggering cause: abandoned retry during shutdown")
    }));
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
    assert!(messages.iter().any(|entry| {
        entry
            .message
            .contains("FemtoSocketHandler dropped 1 records since the last report; triggering cause: abandoned retry during shutdown")
    }));
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
    assert!(messages.iter().any(|entry| {
        entry
            .message
            .contains("FemtoSocketHandler dropped 1 records since the last report; triggering cause: abandoned retry during shutdown")
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
        SocketHandlerConfig::default(),
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
            "FemtoSocketHandler dropped 1 records since the last report",
            Duration::from_secs(5),
        );
        // Release the worker before asserting: an assertion that fails while
        // the sender is still blocked in a rendezvous send would leave
        // `thread::scope` joining a thread this test can no longer release.
        let acknowledged = ack_rx.recv_timeout(Duration::from_secs(5)).is_ok();
        // Closing the channel is what unblocks a worker still waiting to send;
        // without it a failed assertion would hand the scope a join that can
        // never finish.
        drop(ack_rx);
        assert!(acknowledged, "the flush command must be acknowledged");
        assert!(
            messages.iter().any(|entry| entry
                .message
                .contains("FemtoSocketHandler dropped 1 records")),
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
    let mut worker = Worker::new(SocketHandlerConfig::default(), Arc::clone(&signal));
    // Open the interval with a reported drop, then record one it suppresses.
    worker.warner.record_drop();
    worker.warner.warn_if_due(|_| {});
    worker.warner.record_drop();
    assert_eq!(worker.warner.pending_drops(), 1);

    let (command_tx, command_rx) = bounded(1);
    let (ack_tx, ack_rx) = bounded(0);
    let command = match pending {
        PendingAck::Shutdown => SocketCommand::Shutdown(ack_tx),
        PendingAck::FlushAfterAbort => SocketCommand::Flush(ack_tx),
    };
    command_tx.send(command).expect("queue the drain command");

    thread::scope(|scope| {
        let worker = &mut worker;
        scope.spawn(move || worker.drain_pending(&command_rx, None));
        let messages = wait_for_message(
            "FemtoSocketHandler dropped 1 records since the last report",
            Duration::from_secs(5),
        );
        // Release the worker before asserting: an assertion that fails while
        // the worker is blocked in a rendezvous acknowledgement send would
        // leave `thread::scope` joining a thread this test can no longer
        // release.
        let acknowledged = ack_rx.recv_timeout(Duration::from_secs(5)).is_ok();
        // Closing the channel is what unblocks a worker still waiting to send;
        // without it a failed assertion would hand the scope a join that can
        // never finish.
        drop(ack_rx);
        assert!(
            acknowledged,
            "the queued drain command must be acknowledged"
        );
        assert!(
            messages.iter().any(|entry| entry
                .message
                .contains("FemtoSocketHandler dropped 1 records")),
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

/// A flush spends one budget across both halves, not one on each.
///
/// The wait for queue space and the acknowledgement wait used to be given the
/// flush timeout separately, so a flush whose send spent most of the budget
/// still waited another full timeout for the answer. That is the path this
/// pins: the send has to *succeed* late for the two behaviours to differ. (A
/// send that times out returns `false` before the acknowledgement is reached at
/// all, so a permanently full queue cannot distinguish them - hence a worker
/// that reads here rather than one that never does.)
///
/// The worker holds the flush command for most of the budget and then
/// acknowledges promptly. Under one shared deadline the acknowledgement gets
/// only the remainder and fails; under separate deadlines it would get a fresh
/// budget and succeed. The bound sits between those two outcomes.
#[test]
#[serial_test::serial]
fn a_late_flush_send_leaves_too_little_budget_for_the_acknowledgement() {
    let flush_timeout = Duration::from_millis(200);
    let (tx, rx) = bounded(1);
    // A worker that picks the command up only after the send has spent most of
    // the budget, then answers at once. It keeps its receiver alive so the
    // acknowledgement channel stays open and the wait runs to its timeout
    // rather than ending as a disconnect.
    // The worker must leave the record queued for most of the budget, then take
    // it and acknowledge. Sleeping *before* the receive is what makes the send
    // below block; taking the record first would free the slot at once and the
    // send would never spend anything.
    let worker = thread::spawn(move || {
        thread::sleep(flush_timeout * 4 / 5);
        rx.recv()
            .expect("receive the occupying record once the send is waiting");
        let command = rx.recv().expect("receive the flush command");
        let SocketCommand::Flush(ack) = command else {
            panic!("the queued command must be the flush this test sent");
        };
        // Answer too late for the remainder the send left behind.
        thread::sleep(flush_timeout / 4);
        let _ = ack.send(());
        // Stay alive so the channel is not disconnected while the caller waits.
        thread::sleep(flush_timeout * 2);
    });
    // Block the single slot, so the send below waits until the worker frees it.
    tx.send(SocketCommand::Record(FemtoLogRecord::new(
        "test.logger",
        FemtoLevel::Info,
        "occupy the only slot",
    )))
    .expect("fill the queue so the flush command has to wait");

    let started = Instant::now();
    let acknowledged = flush_queue(&tx, flush_timeout);
    let elapsed = started.elapsed();

    assert!(
        !acknowledged,
        "the acknowledgement arrived only after {elapsed:?}, past the one \
         {flush_timeout:?} budget a flush is allowed; waiting {flush_timeout:?} \
         for the send and then another for the answer is the two-budget \
         behaviour this test exists to prevent"
    );
    assert!(
        elapsed < flush_timeout * 3 / 2,
        "flush took {elapsed:?}, longer than one budget plus scheduling slack"
    );
    worker.join().expect("the test worker must not panic");
}
