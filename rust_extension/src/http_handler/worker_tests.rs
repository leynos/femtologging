//! Unit tests for the HTTP worker's own helpers.
//!
//! Response classification has its own file, `response_classification_tests.rs`,
//! and is not repeated here: what is left in the worker is the authentication
//! encoding that no other module owns.

use super::*;
use std::{
    io::Read,
    net::TcpListener,
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

use crate::{level::FemtoLevel, shutdown_signal::ShutdownSignal};

#[test]
fn base64_encode_basic() {
    assert_eq!(base64_encode(b"user:pass"), "dXNlcjpwYXNz");
    assert_eq!(base64_encode(b"a"), "YQ==");
    assert_eq!(base64_encode(b"ab"), "YWI=");
    assert_eq!(base64_encode(b"abc"), "YWJj");
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
            .contains("FemtoHTTPHandler dropped 1 records during shutdown")
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
            .contains("FemtoHTTPHandler dropped 1 records during shutdown")
    }));
}
