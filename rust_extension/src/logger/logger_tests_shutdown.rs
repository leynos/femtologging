//! Two-phase shutdown tests for the FemtoLogger worker loop.
//!
//! Exercises `should_shutdown_now`, `shutdown_and_drain`, and the
//! `worker_thread_loop` under sustained load to verify prompt exit
//! and complete record processing.

use super::*;
use rstest::{fixture, rstest};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

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

#[fixture]
fn collecting_handler() -> Arc<CollectingHandler> {
    Arc::new(CollectingHandler::new())
}

#[rstest]
fn shutdown_and_drain_processes_all_records_in_order(collecting_handler: Arc<CollectingHandler>) {
    let (tx, rx) = crossbeam_channel::bounded(8);

    for i in 0..5 {
        tx.send(QueuedRecord {
            record: FemtoLogRecord::new("core", FemtoLevel::Info, &format!("msg-{i}")),
            handlers: vec![collecting_handler.clone() as Arc<dyn FemtoHandlerTrait>],
        })
        .expect("Failed to enqueue record");
    }

    FemtoLogger::shutdown_and_drain(&rx);

    let collected = collecting_handler.collected();
    let msgs: Vec<&str> = collected.iter().map(|r| r.message()).collect();
    assert_eq!(msgs, vec!["msg-0", "msg-1", "msg-2", "msg-3", "msg-4"]);
}

#[rstest]
fn shutdown_and_drain_leaves_channel_empty(collecting_handler: Arc<CollectingHandler>) {
    let (tx, rx) = crossbeam_channel::bounded(4);
    let handler: Arc<dyn FemtoHandlerTrait> = collecting_handler as Arc<dyn FemtoHandlerTrait>;

    tx.send(QueuedRecord {
        record: FemtoLogRecord::new("core", FemtoLevel::Info, "a"),
        handlers: vec![handler.clone()],
    })
    .expect("Failed to enqueue record");
    tx.send(QueuedRecord {
        record: FemtoLogRecord::new("core", FemtoLevel::Info, "b"),
        handlers: vec![handler],
    })
    .expect("Failed to enqueue record");

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
    for i in 0..record_count {
        tx.send(QueuedRecord {
            record: FemtoLogRecord::new("core", FemtoLevel::Info, &format!("{i}")),
            handlers: vec![handler_trait.clone()],
        })
        .expect("Failed to enqueue record");
    }
    // Send the shutdown signal before the worker even starts so
    // Phase 1 picks it up immediately.
    shutdown_tx
        .send(())
        .expect("Failed to send shutdown signal");

    let worker = std::thread::spawn(move || {
        FemtoLogger::worker_thread_loop(rx, shutdown_rx);
    });

    worker.join().expect("Worker thread panicked");

    let collected = handler.collected();
    assert_eq!(
        collected.len(),
        record_count,
        "all pre-queued records must be drained on shutdown"
    );
    let msgs: Vec<String> = collected.iter().map(|r| r.message().to_owned()).collect();
    let expected: Vec<String> = (0..record_count).map(|i| i.to_string()).collect();
    assert_eq!(msgs, expected, "records must be drained in FIFO order");
}

#[test]
fn worker_loop_exits_promptly_under_sustained_traffic() {
    use std::sync::atomic::AtomicBool;

    let (tx, rx) = crossbeam_channel::bounded(64);
    let (shutdown_tx, shutdown_rx) = crossbeam_channel::bounded(1);
    let (done_tx, done_rx) = crossbeam_channel::bounded(1);
    let (started_tx, started_rx) = crossbeam_channel::bounded(1);
    let handler = Arc::new(CountingHandler::with_first_signal(started_tx));
    let handler_trait: Arc<dyn FemtoHandlerTrait> = handler.clone();

    let running = Arc::new(AtomicBool::new(true));
    let producer_running = Arc::clone(&running);
    let producer_handler = handler_trait.clone();

    let producer = std::thread::spawn(move || {
        while producer_running.load(Ordering::Relaxed) {
            let record = QueuedRecord {
                record: FemtoLogRecord::new("core", FemtoLevel::Info, "load"),
                handlers: vec![producer_handler.clone()],
            };
            match tx.try_send(record) {
                Ok(()) => {}
                Err(crossbeam_channel::TrySendError::Full(_)) => {
                    std::thread::yield_now();
                }
                Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                    break;
                }
            }
        }
    });

    let worker = std::thread::spawn(move || {
        FemtoLogger::worker_thread_loop(rx, shutdown_rx);
        done_tx
            .send(())
            .expect("Failed to signal worker completion");
    });

    // Wait until the worker has consumed at least one record.
    started_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("Worker did not start processing records");

    shutdown_tx
        .send(())
        .expect("Failed to send shutdown signal");
    running.store(false, Ordering::Relaxed);
    producer.join().expect("Producer thread panicked");

    done_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("Worker did not exit within 2 s of shutdown signal");
    worker.join().expect("Worker thread panicked");

    // Confirm the handler actually processed records (the sustained
    // traffic should have driven the count well above zero).
    assert!(
        handler.count.load(Ordering::SeqCst) > 0,
        "expected at least one record to be processed"
    );
}
