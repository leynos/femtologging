//! Drain-on-drop and pool-shape tests for the shared dispatch pool.

use super::logger_tests_helpers::{CollectingHandler, collected_messages};
use super::*;
use crate::logger::dispatcher::{Dispatcher, Shard, shared_with_shards_for_test};
use std::sync::Arc;

/// Return a pool a test owns, with its workers already running.
///
/// The pool is returned rather than installed as the process-wide one, so the
/// shard count a test chooses does not change what other tests see.
fn pool_with(shards: usize) -> Dispatcher {
    shared_with_shards_for_test(shards).expect("a test pool should start")
}

#[test]
fn a_pool_hands_out_every_shard_in_turn() {
    let dispatcher = pool_with(3);

    let first = dispatcher.assign_shard() as *const Shard;
    let second = dispatcher.assign_shard() as *const Shard;
    let third = dispatcher.assign_shard() as *const Shard;
    let fourth = dispatcher.assign_shard() as *const Shard;

    assert_ne!(first, second, "consecutive loggers must not share a shard");
    assert_ne!(second, third, "consecutive loggers must not share a shard");
    assert_eq!(
        first, fourth,
        "a pool of three must come back to its first shard on the fourth logger"
    );
}

#[test]
fn every_shard_a_pool_hands_out_is_live() {
    let dispatcher = pool_with(2);
    let handler = Arc::new(CollectingHandler::new());

    for _ in 0..2 {
        let shard = dispatcher.assign_shard();
        let queued = QueuedRecord {
            record: FemtoLogRecord::new("pool", FemtoLevel::Info, "dispatched"),
            handlers: vec![handler.clone() as Arc<dyn FemtoHandlerTrait>],
            #[cfg(feature = "python")]
            context: None,
        };
        assert!(
            shard.admit(Box::new(queued)).is_ok(),
            "a fresh shard should accept a record"
        );
        assert!(
            shard.wait_for_drain(),
            "every shard in a pool must have a worker to answer it"
        );
    }

    assert_eq!(
        collected_messages(&handler).len(),
        2,
        "each shard's worker must dispatch what it was given"
    );
}

/// A worker that has been idle survives being idle.
///
/// The worker waits for work with a bounded `recv_timeout`, which is a liveness
/// tick so the loop notices a dropped queue. Treating the tick's timeout as the
/// end of the shard rather than looping retires every pool worker the first
/// time the process logs nothing for that long, and the damage lands on the
/// *next* flush: its barrier waits for a worker that is no longer running. That
/// made the bug invisible to tests that log, flush and finish, because none of
/// them ever let a worker idle.
///
/// The wait is deliberately longer than the tick. It is the only test that
/// spends real time idle, and it is what makes the difference observable.
#[test]
fn an_idle_worker_keeps_answering() {
    let dispatcher = pool_with(1);
    let shard = dispatcher.assign_shard();
    let idle_tick = crate::logger::dispatcher::idle_tick_for_test();

    std::thread::sleep(idle_tick * 2);

    let handler = Arc::new(CollectingHandler::new());
    let queued = QueuedRecord {
        record: FemtoLogRecord::new("idle", FemtoLevel::Info, "after idling"),
        handlers: vec![handler.clone() as Arc<dyn FemtoHandlerTrait>],
        #[cfg(feature = "python")]
        context: None,
    };
    assert!(
        shard.admit(Box::new(queued)).is_ok(),
        "a shard whose worker idled must still accept a record"
    );
    assert!(
        shard.wait_for_drain(),
        "a worker that has been idle must still answer a barrier; a worker that \
         exited on its idle tick leaves the flush waiting for nobody"
    );
    assert_eq!(
        collected_messages(&handler),
        vec!["after idling"],
        "an idled worker must dispatch the record it is next given"
    );
}

#[test]
fn one_shard_refuses_records_beyond_its_bound() {
    // Started directly at depth 1 rather than taken from a pool. A pool
    // shard's depth is `RECORDS_PER_LOGGER * shard_count`, so a pool-based
    // test cannot reach the bound without a race against its own worker: the
    // first version of this test asserted the bound while holding one permit
    // out of 1024, and passed only because it never reached the refusal. Depth
    // is the only input the bound depends on, so a one-deep shard exercises
    // the same refusal path.
    let shard = Shard::start(0, 1).expect("a test shard should start");
    // A handler that never returns, so the single in-flight permit the shard
    // hands out is never given back: the bound is reached without a race.
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();

    struct BlockingHandler {
        started: std::sync::mpsc::Sender<()>,
        release: parking_lot::Mutex<std::sync::mpsc::Receiver<()>>,
    }

    impl FemtoHandlerTrait for BlockingHandler {
        fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
            let _ = self.started.send(());
            let _ = self.release.lock().recv();
            Ok(())
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    let handler = Arc::new(BlockingHandler {
        started: started_tx,
        release: parking_lot::Mutex::new(release_rx),
    }) as Arc<dyn FemtoHandlerTrait>;

    let queued = |message: &str| QueuedRecord {
        record: FemtoLogRecord::new("bounded", FemtoLevel::Info, message),
        handlers: vec![Arc::clone(&handler)],
        #[cfg(feature = "python")]
        context: None,
    };
    assert!(
        shard.admit(Box::new(queued("held"))).is_ok(),
        "the first record fits"
    );
    started_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("the worker should start handling the first record");

    let refused = shard.admit(Box::new(queued("refused")));
    assert!(
        refused.is_err(),
        "a shard holding its whole budget must refuse the next record instead of queueing it"
    );
    let refused = refused.expect_err("refusal was asserted above");
    assert_eq!(refused.record.message(), "refused");

    release_tx
        .send(())
        .expect("the worker should still be waiting");
}
