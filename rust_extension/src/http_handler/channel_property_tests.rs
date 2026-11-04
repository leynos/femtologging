//! Channel properties behind the HTTP handler's shutdown hand-off.
//!
//! The handler raises the shutdown signal before it queues the shutdown
//! command, so a worker can leave on the signal without ever reading that
//! command. The command owns the only real acknowledgement sender, which makes
//! what happens to an unread command the difference between `close` returning
//! at once and `close` reporting a finished worker as abandoned.
//!
//! These are properties of the queue rather than threaded tests, because a
//! threaded test can only race for the window instead of holding it.

use std::time::Duration;

use rstest::rstest;

use crate::http_handler::queue::HTTPCommand;

/// An unread shutdown acknowledgement disconnects with the last sender.
///
/// If the handler kept its sender alive while waiting, the queue and the
/// unread command stayed allocated and the acknowledgement channel stayed
/// open: the wait below could neither receive nor observe a disconnect, so it
/// sat out the whole budget and abandoned a worker that had already finished.
/// Dropping the handler's sender first is what turns that into the disconnect
/// the wait already reads as "the worker has left".
#[rstest]
fn an_unread_command_acknowledgement_disconnects_with_its_last_sender() {
    let (tx, rx) = crossbeam_channel::bounded::<HTTPCommand>(1);
    let (ack_tx, ack_rx) = crossbeam_channel::bounded::<()>(1);
    tx.send(HTTPCommand::Shutdown(ack_tx))
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
    let (tx, rx) = crossbeam_channel::bounded::<HTTPCommand>(1);
    let (ack_tx, ack_rx) = crossbeam_channel::bounded::<()>(1);
    tx.send(HTTPCommand::Shutdown(ack_tx))
        .expect("queue the shutdown command");
    drop(tx);

    match rx.try_recv() {
        Ok(HTTPCommand::Shutdown(ack)) => ack.send(()).expect("send the acknowledgement"),
        other => panic!("the queued command must still be readable, got {other:?}"),
    }
    assert_eq!(
        ack_rx.recv_timeout(Duration::from_millis(50)),
        Ok(()),
        "a worker that reads the queued command must still be able to answer"
    );
}
