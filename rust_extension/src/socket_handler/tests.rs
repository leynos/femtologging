//! Tests for the socket handler implementation.

use std::{
    io::{self, Read},
    net::{Shutdown, SocketAddr, TcpListener},
    sync::{Arc, Barrier, mpsc},
    thread,
    time::{Duration, Instant},
};

use rstest::{fixture, rstest};
use serde::Deserialize;

use crate::{
    handler::{FemtoHandlerTrait, HandlerError},
    handlers::{HandlerBuildError, HandlerBuilderTrait, socket_builder::SocketHandlerBuilder},
    level::FemtoLevel,
    log_record::FemtoLogRecord,
    shutdown_signal::ShutdownSignal,
    socket_handler::{FemtoSocketHandler, SocketHandlerConfig},
};

use super::{
    backoff::BackoffState,
    config::BackoffPolicy,
    serialize::{frame_payload, serialize_record},
    transport::{SocketTransport, TcpTransport, TlsOptions, connect_transport},
};

// Fixtures and helpers arrange state, so they return Result and leave the
// verdict to the calling test.
#[fixture]
fn tcp_listener() -> io::Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", 0))
}

/// Receives the result of reading one length-prefixed frame from the mock server.
type FrameRx = mpsc::Receiver<io::Result<Vec<u8>>>;

fn spawn_single_frame_server(
    listener: TcpListener,
    gate: Option<Arc<Barrier>>,
) -> io::Result<(SocketAddr, FrameRx)> {
    let addr = listener.local_addr()?;
    let (notify_tx, notify_rx) = mpsc::channel();
    thread::spawn(move || {
        // Deliver the outcome to the test; a dropped receiver means the
        // test has already failed, so the send error is ignored.
        let _ = notify_tx.send(read_single_frame(&listener, gate));
    });
    Ok((addr, notify_rx))
}

/// Accept one connection and read a single length-prefixed frame.
fn read_single_frame(listener: &TcpListener, gate: Option<Arc<Barrier>>) -> io::Result<Vec<u8>> {
    let (mut stream, _) = listener.accept()?;
    if let Some(barrier) = gate {
        barrier.wait();
    }
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload)?;
    Ok(payload)
}

#[rstest]
fn builder_requires_transport() {
    let builder = SocketHandlerBuilder::new();
    let err = builder
        .build_inner()
        .expect_err("transport must be required");
    assert!(matches!(err, HandlerBuildError::InvalidConfig(msg) if msg.contains("transport")));
}

#[rstest]
fn builder_rejects_zero_capacity() {
    let err = SocketHandlerBuilder::new()
        .with_tcp("127.0.0.1", 9020)
        .with_capacity(0)
        .build_inner()
        .expect_err("zero capacity must fail");
    assert!(matches!(err, HandlerBuildError::InvalidConfig(msg) if msg.contains("capacity")));
}

#[rstest]
fn builder_rejects_tls_for_unix() {
    let err = SocketHandlerBuilder::new()
        .with_unix_path("/tmp/femtologging.sock")
        .with_tls(Some("example.com".into()), false)
        .build_inner()
        .expect_err("tls should be invalid for unix sockets");
    assert!(matches!(err, HandlerBuildError::InvalidConfig(msg) if msg.contains("tls")));
}

/// Record payload decoded from the socket handler's serialized frame.
#[derive(Debug, Deserialize)]
struct Payload {
    /// Name of the logger that emitted the record.
    logger: String,
    /// Text form of the record's severity.
    level: String,
    /// Message text emitted by the logger.
    message: String,
}

fn build_tcp_handler(addr: SocketAddr) -> Result<FemtoSocketHandler, HandlerBuildError> {
    SocketHandlerBuilder::new()
        .with_tcp(addr.ip().to_string(), addr.port())
        .build_inner()
}

fn send_info_record(handler: &mut FemtoSocketHandler, message: &str) -> Result<(), HandlerError> {
    handler.handle(FemtoLogRecord::new("test", FemtoLevel::Info, message))
}

/// Accept one TLS client, report its hello, then hold the handshake open.
///
/// The release sender drops the peer after the close assertion, letting the
/// detached worker leave its in-flight handshake and discard queued records.
///
/// # Examples
///
/// ```ignore
/// let (addr, hello, release) = spawn_stalled_tls_peer(listener)?;
/// hello.recv_timeout(Duration::from_secs(2))?;
/// release.send(()).expect("release the stalled peer");
/// ```
fn spawn_stalled_tls_peer(
    listener: TcpListener,
) -> io::Result<(SocketAddr, mpsc::Receiver<()>, mpsc::Sender<()>)> {
    let addr = listener.local_addr()?;
    let (hello_tx, hello_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut tls_record_header = [0_u8; 5];
        if stream.read_exact(&mut tls_record_header).is_err() {
            return;
        }
        let _ = hello_tx.send(());
        let _ = release_rx.recv();
    });
    Ok((addr, hello_rx, release_tx))
}

fn recv_payload(
    notify_rx: &FrameRx,
    expectation: &str,
) -> Result<Payload, Box<dyn std::error::Error>> {
    let payload = notify_rx
        .recv_timeout(Duration::from_secs(2))
        .map_err(|err| format!("{expectation}: {err}"))??;
    Ok(rmp_serde::from_slice(&payload)?)
}

/// Arrange a single-frame server plus a connected TCP handler that has
/// already queued one record with the given message.
fn arrange_sent_record(
    listener: TcpListener,
    gate: Option<Arc<Barrier>>,
    message: &str,
) -> Result<(FemtoSocketHandler, FrameRx), Box<dyn std::error::Error>> {
    let (addr, notify_rx) = spawn_single_frame_server(listener, gate)?;
    let mut handler = build_tcp_handler(addr)?;
    send_info_record(&mut handler, message)?;
    Ok((handler, notify_rx))
}

#[rstest]
fn sends_records_over_tcp(tcp_listener: io::Result<TcpListener>) {
    let tcp_listener = tcp_listener.expect("bind ephemeral listener");
    let (mut handler, notify_rx) =
        arrange_sent_record(tcp_listener, None, "message").expect("arrange handler and record");

    let decoded = recv_payload(&notify_rx, "payload received").expect("decode payload");
    assert_eq!(decoded.logger, "test");
    assert_eq!(decoded.level, "INFO");
    assert_eq!(decoded.message, "message");

    handler.close();
}

#[rstest]
fn handler_flushes_pending_records_on_close(tcp_listener: io::Result<TcpListener>) {
    let tcp_listener = tcp_listener.expect("bind ephemeral listener");
    let barrier = Arc::new(Barrier::new(2));
    let (mut handler, notify_rx) =
        arrange_sent_record(tcp_listener, Some(barrier.clone()), "close")
            .expect("arrange handler and record");

    let close_budget = SocketHandlerConfig::default().write_timeout;
    let started = Instant::now();
    handler.close();
    let elapsed = started.elapsed();
    assert!(
        elapsed < close_budget + Duration::from_millis(500),
        "close took {elapsed:?}, allowance was {:?}",
        close_budget + Duration::from_millis(500)
    );
    barrier.wait();

    let decoded = recv_payload(&notify_rx, "payload received after close").expect("decode payload");
    assert_eq!(decoded.message, "close");
}

#[path = "close_tests.rs"]
mod close;

// Contracts for what the worker does with the shutdown signal, as opposed to
// what the handler does with the acknowledgement.
#[path = "drain_tests.rs"]
mod drain;

#[path = "unit_tests.rs"]
mod unit;

#[rstest]
fn tls_handshake_respects_timeout(tcp_listener: io::Result<TcpListener>) {
    let tcp_listener = tcp_listener.expect("bind ephemeral listener");
    let addr = tcp_listener.local_addr().expect("listener has address");
    let (accepted_tx, accepted_rx) = mpsc::channel();
    thread::spawn(move || {
        let (stream, _) = tcp_listener.accept().expect("accept connection");
        accepted_tx.send(()).expect("signal accepted");
        // Keep the TCP connection open without speaking TLS.
        // This simulates a peer that stalls during the handshake.
        thread::sleep(Duration::from_secs(2));
        drop(stream);
    });

    let (result_tx, result_rx) = mpsc::channel();
    let host = addr.ip().to_string();
    let port = addr.port();
    thread::spawn(move || {
        let transport = SocketTransport::Tcp(TcpTransport {
            host,
            port,
            tls: Some(TlsOptions {
                domain: "localhost".into(),
                insecure_skip_verify: true,
            }),
        });
        // No shutdown is under way, so the configured timeout is the only
        // bound on the handshake: this test is about the timeout, not the
        // signal, which is why the signal is left running.
        let signal = ShutdownSignal::new();
        let start = Instant::now();
        let result = connect_transport(&transport, Duration::from_millis(250), &signal);
        let elapsed = start.elapsed();
        let ok = result.is_ok();
        drop(result);
        result_tx
            .send((ok, elapsed))
            .expect("handshake duration should send");
    });

    accepted_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("connection must be accepted");
    let (ok, elapsed) = result_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("handshake result should arrive");
    assert!(!ok, "handshake should fail for stalled peer");
    assert!(
        elapsed < Duration::from_secs(2),
        "handshake should respect timeout, elapsed {:?}",
        elapsed
    );
}

#[rstest]
#[serial_test::serial]
fn close_interrupts_backoff_after_a_failed_socket_write(tcp_listener: io::Result<TcpListener>) {
    let listener = tcp_listener.expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("listener has address");
    let (reset_tx, reset_rx) = mpsc::channel();
    thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept client connection");
        stream
            .shutdown(Shutdown::Both)
            .expect("reset the peer connection");
        reset_tx.send(()).expect("signal the reset peer");
    });

    crate::handlers::file::test_support::install_test_logger();
    let close_budget = Duration::from_millis(500);
    let mut handler = FemtoSocketHandler::with_config(SocketHandlerConfig {
        capacity: 1,
        connect_timeout: Duration::from_secs(2),
        write_timeout: close_budget,
        max_frame_size: 32 * 1024 * 1024,
        transport: SocketTransport::Tcp(TcpTransport {
            host: addr.ip().to_string(),
            port: addr.port(),
            tls: None,
        }),
        backoff: BackoffPolicy {
            base: Duration::from_secs(30),
            cap: Duration::from_secs(30),
            deadline: Duration::from_secs(60),
            ..BackoffPolicy::default()
        },
        ..SocketHandlerConfig::default()
    });
    let backoffs = handler.wait_notification();
    let large_message = "x".repeat(16 * 1024 * 1024);
    send_info_record(&mut handler, &large_message).expect("queue large record");
    reset_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("server should accept and reset the connection");
    backoffs
        .recv_timeout(Duration::from_secs(5))
        .expect("failed write should enter retry backoff");
    let worker_logs = crate::handlers::file::test_support::wait_for_message(
        "FemtoSocketHandler write failed",
        Duration::from_secs(2),
    );
    assert!(
        worker_logs
            .iter()
            .any(|entry| entry.message.contains("FemtoSocketHandler write failed")),
        "the peer reset must fail the record write; got {worker_logs:?}"
    );

    let started = Instant::now();
    handler.close();
    let elapsed = started.elapsed();
    assert!(
        elapsed < close_budget + Duration::from_millis(500),
        "close should interrupt the long write backoff: took {elapsed:?}"
    );
    let close_logs = crate::handlers::file::test_support::take_logged_messages();
    assert!(
        close_logs
            .iter()
            .all(|entry| !entry.message.contains("abandoning it")),
        "close during backoff should be acknowledged, got {close_logs:?}"
    );
}
