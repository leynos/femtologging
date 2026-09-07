//! Two-phase shutdown tests for the `FemtoLogger` worker loop.
//!
//! Exercises `should_shutdown_now`, `shutdown_and_drain`, and the
//! `worker_thread_loop` drain-on-shutdown guarantee.

use std::sync::Arc;

use rstest::rstest;

use super::{
    super::logger_tests_helpers::{collected_messages, collecting_handler, enqueue_records},
    *,
};

// ------------------------------------------------------------------
// should_shutdown_now
// ------------------------------------------------------------------

/// Channel state variants exercised by the parametrized shutdown
/// detection test.
enum ShutdownState {
    /// A shutdown message has been sent on the channel.
    MessageSent,
    /// The sender has been dropped, disconnecting the channel.
    Disconnected,
    /// The channel is open but empty.
    Empty,
}

#[rstest(
    state,
    expected,
    case::message_sent(ShutdownState::MessageSent, true),
    case::disconnected(ShutdownState::Disconnected, true),
    case::empty(ShutdownState::Empty, false)
)]
fn should_shutdown_now_cases(state: ShutdownState, expected: bool) {
    let (tx, rx) = crossbeam_channel::bounded::<()>(1);
    match state {
        ShutdownState::MessageSent => {
            tx.send(()).expect("Failed to send shutdown signal");
        }
        ShutdownState::Disconnected => drop(tx),
        ShutdownState::Empty => { /* keep tx alive, channel stays open */ }
    }
    assert_eq!(FemtoLogger::should_shutdown_now(&rx), expected);
}

// ------------------------------------------------------------------
// shutdown_and_drain
// ------------------------------------------------------------------

#[rstest]
fn shutdown_and_drain_processes_all_records_in_order(collecting_handler: Arc<CollectingHandler>) {
    let (tx, rx) = crossbeam_channel::bounded(8);
    let handler = collecting_handler.clone() as Arc<dyn FemtoHandlerTrait>;
    enqueue_records(
        &tx,
        &handler,
        &["msg-0", "msg-1", "msg-2", "msg-3", "msg-4"],
    )
    .expect("Failed to enqueue records");

    FemtoLogger::shutdown_and_drain(&rx);

    assert_eq!(
        collected_messages(&collecting_handler),
        vec!["msg-0", "msg-1", "msg-2", "msg-3", "msg-4"]
    );
}

#[rstest]
fn shutdown_and_drain_leaves_channel_empty(collecting_handler: Arc<CollectingHandler>) {
    let (tx, rx) = crossbeam_channel::bounded(4);
    let handler: Arc<dyn FemtoHandlerTrait> = collecting_handler as Arc<dyn FemtoHandlerTrait>;
    enqueue_records(&tx, &handler, &["a", "b"]).expect("Failed to enqueue records");

    FemtoLogger::shutdown_and_drain(&rx);

    assert!(
        rx.try_recv().is_err(),
        "expected channel to be empty after drain"
    );
}

// ------------------------------------------------------------------
// worker_thread_loop — stress / behavioural
// ------------------------------------------------------------------

#[test]
fn worker_loop_drains_all_queued_records_on_shutdown() {
    let (tx, rx) = crossbeam_channel::bounded(128);
    let (shutdown_tx, shutdown_rx) = crossbeam_channel::bounded(1);
    let handler = Arc::new(CollectingHandler::new());
    let handler_trait: Arc<dyn FemtoHandlerTrait> = handler.clone();

    // Pre-fill the channel with a known sequence *before* starting
    // the worker so every record is guaranteed to be queued when the
    // shutdown signal arrives.
    let record_count: usize = 50;
    let messages: Vec<String> = (0..record_count).map(|i| i.to_string()).collect();
    let message_refs: Vec<&str> = messages.iter().map(String::as_str).collect();
    enqueue_records(&tx, &handler_trait, &message_refs).expect("Failed to enqueue records");
    // Send the shutdown signal before the worker even starts so
    // Phase 1 picks it up immediately.
    shutdown_tx
        .send(())
        .expect("Failed to send shutdown signal");

    let worker = std::thread::spawn(move || {
        FemtoLogger::worker_thread_loop(&rx, &shutdown_rx);
    });

    worker.join().expect("Worker thread panicked");

    let msgs = collected_messages(&handler);
    assert_eq!(
        msgs.len(),
        record_count,
        "all pre-queued records must be drained on shutdown"
    );
    assert_eq!(msgs, messages, "records must be drained in FIFO order");
}
