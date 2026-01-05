//! Tests for the socket handler implementation.

use std::{
    io::Read,
    net::{SocketAddr, TcpListener},
    sync::{Arc, Barrier, mpsc},
    thread,
    time::{Duration, Instant},
};

use rstest::{fixture, rstest};
use serde::Deserialize;

use crate::{
    handler::FemtoHandlerTrait,
    handlers::{HandlerBuildError, HandlerBuilderTrait, socket_builder::SocketHandlerBuilder},
    level::FemtoLevel,
    log_record::FemtoLogRecord,
    socket_handler::FemtoSocketHandler,
};

use super::{
    backoff::BackoffState,
    config::BackoffPolicy,
    serialise::{frame_payload, serialise_record},
    transport::{SocketTransport, TcpTransport, TlsOptions, connect_transport},
};

#[fixture]
fn tcp_listener() -> TcpListener {
    TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral listener")
}

fn spawn_single_frame_server(
    listener: TcpListener,
    gate: Option<Arc<Barrier>>,
) -> (SocketAddr, mpsc::Receiver<Vec<u8>>) {
    let addr = listener.local_addr().expect("listener has address");
    let (notify_tx, notify_rx) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept connection");
        if let Some(barrier) = gate {
            barrier.wait();
        }
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).expect("read frame len");
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload).expect("read payload");
        notify_tx.send(payload).expect("send payload");
    });
    (addr, notify_rx)
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

#[derive(Debug, Deserialize)]
struct Payload {
    logger: String,
    level: String,
    message: String,
}

fn build_tcp_handler(addr: SocketAddr) -> FemtoSocketHandler {
    SocketHandlerBuilder::new()
        .with_tcp(addr.ip().to_string(), addr.port())
        .build_inner()
        .expect("build handler")
}

fn send_info_record(handler: &mut FemtoSocketHandler, message: &str) {
    handler
        .handle(FemtoLogRecord::new("test", FemtoLevel::Info, message))
        .expect("send record");
}

fn recv_payload(notify_rx: &mpsc::Receiver<Vec<u8>>, expectation: &str) -> Payload {
    let payload = notify_rx
        .recv_timeout(Duration::from_secs(2))
        .expect(expectation);
    rmp_serde::from_slice(&payload).expect("decode payload")
}

#[rstest]
fn sends_records_over_tcp(tcp_listener: TcpListener) {
    let (addr, notify_rx) = spawn_single_frame_server(tcp_listener, None);
    let mut handler = build_tcp_handler(addr);
    send_info_record(&mut handler, "message");

    let decoded = recv_payload(&notify_rx, "payload received");
    assert_eq!(decoded.logger, "test");
    assert_eq!(decoded.level, "INFO");
    assert_eq!(decoded.message, "message");

    handler.close();
}

#[rstest]
fn handler_flushes_pending_records_on_close(tcp_listener: TcpListener) {
    let barrier = Arc::new(Barrier::new(2));
    let (addr, notify_rx) = spawn_single_frame_server(tcp_listener, Some(barrier.clone()));
    let mut handler = build_tcp_handler(addr);
    send_info_record(&mut handler, "close");

    handler.close();
    barrier.wait();

    let decoded = recv_payload(&notify_rx, "payload received after close");
    assert_eq!(decoded.message, "close");
}

#[rstest]
fn tls_handshake_respects_timeout(tcp_listener: TcpListener) {
    let addr = tcp_listener.local_addr().unwrap();
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
        let start = Instant::now();
        let result = connect_transport(&transport, Duration::from_millis(250));
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
fn frame_payload_enforces_limit() {
    let payload = vec![0u8; 32];
    let framed = frame_payload(&payload, 16);
    assert!(
        framed.is_none(),
        "payload larger than limit must be rejected",
    );
}

#[rstest]
fn frame_payload_prefixes_length() {
    let payload = vec![1u8, 2, 3];
    let framed = frame_payload(&payload, 16).expect("payload fits frame");
    assert_eq!(&framed[..4], &3u32.to_be_bytes());
    assert_eq!(&framed[4..], payload);
}

#[rstest]
fn serialise_record_round_trips() {
    let record = FemtoLogRecord::new("logger", FemtoLevel::Info, "hello");
    let payload = serialise_record(&record).expect("serialise record");
    let decoded: Payload = rmp_serde::from_slice(&payload).expect("decode payload");
    assert_eq!(decoded.logger, "logger");
    assert_eq!(decoded.level, "INFO");
    assert_eq!(decoded.message, "hello");
}

#[rstest]
fn backoff_enforces_minimum_sleep() {
    let mut policy = BackoffPolicy::default();
    policy.base = Duration::from_millis(0);
    policy.cap = Duration::from_millis(0);
    policy.deadline = Duration::from_millis(50);
    let mut backoff = BackoffState::new(policy);
    let now = Instant::now();
    let sleep = backoff
        .next_sleep(now)
        .expect("first backoff value must exist");
    assert!(
        sleep >= Duration::from_millis(10),
        "sleep {:?} should respect minimum",
        sleep
    );
}

#[rstest]
fn backoff_respects_deadline() {
    let mut policy = BackoffPolicy::default();
    policy.base = Duration::from_millis(10);
    policy.cap = Duration::from_millis(10);
    policy.deadline = Duration::from_millis(20);
    let mut backoff = BackoffState::new(policy);
    let now = Instant::now();
    assert!(backoff.next_sleep(now).is_some());
    let expired = now + Duration::from_millis(25);
    assert!(backoff.next_sleep(expired).is_none());
}
