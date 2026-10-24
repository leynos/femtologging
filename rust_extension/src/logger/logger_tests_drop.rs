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

/// A handler that takes a second handler's lock while the drop runs its own.
///
/// The drop drains its shard, so anything the shard calls while draining must
/// not need a lock the drop is holding: if it did, this handler would block
/// until the drain's barrier timed out and the record would be lost.
#[test]
fn drop_does_not_hold_the_handler_list_while_draining() {
    use parking_lot::Mutex as PoisonlessMutex;

    /// Handler that acquires a shared lock while the logger is dropped.
    struct LockTakingHandler {
        /// Lock observed after drop to prove dispatch reached this handler.
        lock: Arc<PoisonlessMutex<usize>>,
    }

    impl FemtoHandlerTrait for LockTakingHandler {
        fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
            *self.lock.lock() += 1;
            Ok(())
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    let lock = Arc::new(PoisonlessMutex::new(0));
    let logger =
        FemtoLogger::new("drop-lock".to_string()).expect("test logger should be constructible");
    logger
        .add_handler(Arc::new(LockTakingHandler {
            lock: Arc::clone(&lock),
        }) as Arc<dyn FemtoHandlerTrait>)
        .expect("native lock-taking test handler should register");

    logger.log(FemtoLevel::Info, "guarded");
    drop(logger);

    assert_eq!(
        *lock.lock(),
        1,
        "the drop must let the dispatch worker reach the handlers"
    );
}
