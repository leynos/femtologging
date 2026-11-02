//! Unit tests for socket frame serialization and retry backoff.

use std::time::{Duration, Instant};

use rstest::rstest;

use super::*;
use crate::socket_handler::transport::{ActiveConnection, UnixTransport};
use crate::socket_handler::worker::SocketCommand;

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
fn serialize_record_round_trips() {
    let record = FemtoLogRecord::new("logger", FemtoLevel::Info, "hello");
    let payload = serialize_record(&record).expect("serialize record");
    let decoded: Payload = rmp_serde::from_slice(&payload).expect("decode payload");
    assert_eq!(decoded.logger, "logger");
    assert_eq!(decoded.level, "INFO");
    assert_eq!(decoded.message, "hello");
}

#[rstest]
fn backoff_enforces_minimum_sleep() {
    let policy = BackoffPolicy {
        base: Duration::from_millis(0),
        cap: Duration::from_millis(0),
        deadline: Duration::from_millis(50),
        ..BackoffPolicy::default()
    };
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
    let policy = BackoffPolicy {
        base: Duration::from_millis(10),
        cap: Duration::from_millis(10),
        deadline: Duration::from_millis(20),
        ..BackoffPolicy::default()
    };
    let mut backoff = BackoffState::new(policy);
    let now = Instant::now();
    assert!(backoff.next_sleep(now).is_some());
    let expired = now + Duration::from_millis(25);
    assert!(backoff.next_sleep(expired).is_none());
}

/// A partial write reports its count, so a caller can reconsider its budget.
///
/// The frame here is far larger than the loopback buffer and nothing reads the
/// other end, so one call cannot take all of it. The assertion is that the
/// count is smaller than the buffer, not that it equals some particular
/// figure: how much a socket accepts before blocking depends on the host.
#[rstest]
fn write_some_reports_partial_progress() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind a peer");
    let addr = listener.local_addr().expect("listener has address");
    let stream = std::net::TcpStream::connect(addr).expect("connect to peer");
    let _peer = listener.accept().expect("accept the connection").0;
    let mut connection = ActiveConnection::PlainTcp(stream);
    // A blocking socket write is bounded per internal wait, not per call, so
    // without a timeout this very test would hang on the buffer it means to
    // measure. The product path always sets one before writing for that reason.
    connection
        .set_write_timeout(Duration::from_millis(100))
        .expect("a connected socket should accept a write timeout");
    let frame = vec![0_u8; 4 * 1024 * 1024];

    let written = connection
        .write_some(&frame)
        .expect("a connected socket should accept some bytes");

    assert!(
        written > 0 && written < frame.len(),
        "a write larger than the send buffer must report a partial count, got \
         {written} of {}",
        frame.len()
    );
}

/// A Unix connect does not begin once the shutdown deadline has passed.
///
/// The connect itself cannot be interrupted, so the contract the handler can
/// keep is the narrower one: an attempt that has not started when the budget
/// runs out is not started. The path here does not exist, so a connect that
/// was attempted would report `NotFound`; the error under test is the budget's
/// `TimedOut`, which is what tells the two apart.
#[cfg(unix)]
#[rstest]
fn unix_connect_is_refused_once_the_budget_is_spent() {
    let signal = ShutdownSignal::new();
    signal.begin_shutdown(Instant::now());
    let transport = SocketTransport::Unix(UnixTransport {
        path: "/tmp/femtologging-budget-never-used.sock".into(),
    });

    let Err(err) = connect_transport(&transport, Duration::from_secs(1), &signal) else {
        panic!("an expired budget must not begin a connect");
    };

    assert_eq!(
        err.kind(),
        std::io::ErrorKind::TimedOut,
        "the refusal must come from the budget rather than from the path: {err}"
    );
}

/// An empty write is not progress, and must not be mistaken for a dead peer.
///
/// `write` on an empty slice returns `Ok(0)`, which the frame writer relies on
/// to mean "nothing left to do" rather than "the connection failed".
#[rstest]
fn write_some_accepts_an_empty_buffer() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind a peer");
    let addr = listener.local_addr().expect("listener has address");
    let stream = std::net::TcpStream::connect(addr).expect("connect to peer");
    let _peer = listener.accept().expect("accept the connection").0;
    let mut connection = ActiveConnection::PlainTcp(stream);

    assert_eq!(
        connection
            .write_some(&[])
            .expect("an empty write should succeed"),
        0
    );
}

/// A queued command's acknowledgement disconnects once every sender is gone.
///
/// This is the channel property `request_shutdown` depends on. The shutdown is
/// carried on the signal as well as on the command, so a worker can leave the
/// loop with that command still unread - and the command owns the only real
/// acknowledgement sender. If the handler kept its own sender alive, the queue
/// stayed allocated, the unread command stayed alive, and the acknowledgement
/// channel stayed open: `close` then waited out its whole budget for a worker
/// that had already stopped. Dropping the handler's sender first is what turns
/// that into the disconnect the wait already reads as "the worker has left".
///
/// Stated as a property of the channel, because a threaded test for it can
/// only race for the window rather than hold it.
#[rstest]
fn an_unread_command_acknowledgement_disconnects_with_its_last_sender() {
    let (tx, rx) = crossbeam_channel::bounded::<SocketCommand>(1);
    let (ack_tx, ack_rx) = crossbeam_channel::bounded::<()>(1);
    tx.send(SocketCommand::Shutdown(ack_tx))
        .expect("queue the shutdown command");
    drop(tx);
    // The worker leaves without reading: dropping the receiver must be enough
    // on its own, because nothing else can now reach the queued command.
    drop(rx);
    assert_eq!(
        ack_rx.recv_timeout(Duration::from_millis(50)),
        Err(crossbeam_channel::RecvTimeoutError::Disconnected),
        "an unread shutdown acknowledgement must disconnect rather than \
         time out, or close waits out its budget for a worker that has left"
    );
}

/// The same queue still delivers a command a live worker has not read yet.
///
/// The regression above could also be satisfied by dropping queued commands
/// too eagerly. A worker still in its loop reads what was queued after the
/// handler's sender is gone, so the acknowledgement must still arrive.
#[rstest]
fn a_queued_command_is_still_read_after_the_last_sender_drops() {
    let (tx, rx) = crossbeam_channel::bounded::<SocketCommand>(1);
    let (ack_tx, ack_rx) = crossbeam_channel::bounded::<()>(1);
    tx.send(SocketCommand::Shutdown(ack_tx))
        .expect("queue the shutdown command");
    drop(tx);

    match rx.try_recv() {
        Ok(SocketCommand::Shutdown(ack)) => ack.send(()).expect("send the acknowledgement"),
        other => panic!("the queued command must still be readable, got {other:?}"),
    }
    assert_eq!(
        ack_rx.recv_timeout(Duration::from_millis(50)),
        Ok(()),
        "a worker that reads the queued command must still be able to answer"
    );
}
