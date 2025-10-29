//! Tests for the socket handler implementation.

use std::{io::Read, net::TcpListener, thread, time::Duration};

use rstest::{fixture, rstest};
use serde::Deserialize;

use crate::{
    handler::FemtoHandlerTrait,
    handlers::{socket_builder::SocketHandlerBuilder, HandlerBuildError, HandlerBuilderTrait},
    log_record::FemtoLogRecord,
};

use super::serialise::frame_payload;

#[fixture]
fn tcp_listener() -> TcpListener {
    TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral listener")
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

#[derive(Debug, Deserialize)]
struct Payload {
    logger: String,
    level: String,
    message: String,
}

#[rstest]
fn sends_records_over_tcp(tcp_listener: TcpListener) {
    let addr = tcp_listener.local_addr().unwrap();
    let (notify_tx, notify_rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = tcp_listener.accept().expect("accept connection");
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).expect("read frame len");
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload).expect("read payload");
        notify_tx.send(payload).expect("send payload");
    });

    let handler = SocketHandlerBuilder::new()
        .with_tcp(addr.ip().to_string(), addr.port())
        .build_inner()
        .expect("build handler");

    handler
        .handle(FemtoLogRecord::new("test", "INFO", "message"))
        .expect("send record");

    let payload = notify_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("payload received");
    let decoded: Payload = rmp_serde::from_slice(&payload).expect("decode payload");
    assert_eq!(decoded.logger, "test");
    assert_eq!(decoded.level, "INFO");
    assert_eq!(decoded.message, "message");

    let mut handler = handler;
    handler.close();
}

#[rstest]
fn frame_payload_enforces_limit() {
    let payload = vec![0u8; 32];
    let framed = frame_payload(&payload, 16);
    assert!(
        framed.is_none(),
        "payload larger than limit must be rejected"
    );
}
