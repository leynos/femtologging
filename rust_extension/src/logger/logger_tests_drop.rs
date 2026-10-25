//! Tests for what `FemtoLogger::drop` waits for and what it must not wait on.
//!
//! A logger's drop drains its shard and joins nothing, so the two things worth
//! asserting are that a record logged immediately before the drop is still
//! delivered, and that the drop does not hold a lock the record's own path
//! needs.

use super::logger_tests_helpers::{CollectingHandler, collected_messages};
use super::*;

#[test]
fn drop_delivers_records_queued_before_it() {
    let handler = Arc::new(CollectingHandler::new());
    let logger =
        FemtoLogger::new("drop-drain".to_string()).expect("test logger should be constructible");
    logger
        .add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>)
        .expect("native collecting handler should register");

    logger.log(FemtoLevel::Info, "before drop");
    drop(logger);

    assert_eq!(
        collected_messages(&handler),
        vec!["before drop"],
        "a record logged before the drop must still be dispatched"
    );
}

#[test]
fn drop_delivers_every_record_queued_before_it_in_order() {
    let handler = Arc::new(CollectingHandler::new());
    let logger =
        FemtoLogger::new("drop-order".to_string()).expect("test logger should be constructible");
    logger
        .add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>)
        .expect("native collecting handler should register");

    for index in 0..8 {
        logger.log(FemtoLevel::Info, &format!("msg-{index}"));
    }
    drop(logger);

    let expected: Vec<String> = (0..8).map(|index| format!("msg-{index}")).collect();
    assert_eq!(
        collected_messages(&handler),
        expected,
        "the shard must drain a logger's records in the order it queued them"
    );
}

/// The drop must not hold the handler list while it drains.
///
/// `Drop` waits for the shard to finish the records this logger queued, and a
/// queued handler may want the logger's handler list — a handler that inspects
/// its own configuration, or one that re-reads the list, takes that lock. If
/// the drop held it while waiting, the handler could never finish, the barrier
/// would time out, and the record would be lost.
///
/// The probe reads the logger's *own* `handlers` lock, not an unrelated one.
/// That distinction is the whole test: taking some other lock would pass no
/// matter what the drop held, which is what an earlier version of this test
/// did.
///
/// The probe samples the *whole* window rather than stopping at the first
/// success, and that is what makes the result meaningful. The window opens just
/// before the drop is spawned, so its early probes may run before the dropping
/// thread is scheduled at all; a test that stopped at the first success would
/// pass against a drop that holds the list for the rest of the window. A drop
/// that took the list holds it until its barrier resolves, and the handler is
/// blocked for the whole window, so the barrier cannot resolve inside it: the
/// lock is therefore observed held by at least the later probes, and the
/// assertion fails.
///
/// The handler is blocked inside `handle` before the drop starts, so the record
/// is in flight and the shard's worker cannot advance past it. The window is
/// well inside the shard's two-second barrier timeout, so a drop that took the
/// list cannot time out and release it part-way through.
#[test]
fn drop_does_not_hold_the_handler_list_while_draining() {
    use crossbeam_channel::{Receiver, Sender, bounded};
    use std::time::{Duration, Instant};

    /// How long the main thread samples the handler list while the drop drains.
    const PROBE_WINDOW: Duration = Duration::from_millis(250);

    /// Handler that blocks inside `handle` until the test releases it.
    struct BlockingHandler {
        /// Signals that dispatch has started and the record is in flight.
        entered: Sender<()>,
        /// Released by the test once the probe window has elapsed.
        release: Receiver<()>,
    }

    impl FemtoHandlerTrait for BlockingHandler {
        fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
            let _ = self.entered.send(());
            let _ = self.release.recv();
            Ok(())
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    let (entered_tx, entered_rx) = bounded::<()>(1);
    let (release_tx, release_rx) = bounded::<()>(1);

    let logger =
        FemtoLogger::new("drop-lock".to_string()).expect("test logger should be constructible");
    let handlers = Arc::clone(&logger.handlers);
    logger
        .add_handler(Arc::new(BlockingHandler {
            entered: entered_tx,
            release: release_rx,
        }) as Arc<dyn FemtoHandlerTrait>)
        .expect("native blocking test handler should register");

    logger.log(FemtoLevel::Info, "guarded");
    entered_rx.recv_timeout(Duration::from_secs(5)).expect(
        "the record must reach the handler before the drop starts, so the drop \
         has something in flight to wait for",
    );

    let dropper = std::thread::spawn(move || drop(logger));
    let deadline = Instant::now() + PROBE_WINDOW;
    let mut observed_held = false;
    while Instant::now() < deadline {
        // Drop each guard immediately: the probe is about observation, not
        // contention. Holding it would make the next probe fail on our own
        // account.
        observed_held |= handlers.try_write().is_none();
        std::thread::sleep(Duration::from_millis(1));
    }
    release_tx
        .send(())
        .expect("the handler must still be waiting to be released");
    dropper.join().expect("the dropping thread must not panic");

    assert!(
        !observed_held,
        "the drop must not hold the handler list while it drains: the list was \
         observed locked during a {PROBE_WINDOW:?} window in which the drop was \
         waiting for a blocked handler, which only a drop that took the list \
         can cause"
    );
}
