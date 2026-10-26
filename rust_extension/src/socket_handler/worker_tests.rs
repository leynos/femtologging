//! Tests for socket-worker shutdown command ordering.

use super::*;
use std::{
    io::Read,
    net::TcpListener,
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

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
            .contains("FemtoSocketHandler dropped 1 records during shutdown")
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
            .contains("FemtoSocketHandler dropped 1 records during shutdown")
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
            .contains("FemtoSocketHandler dropped 1 records during shutdown")
    }));
}
