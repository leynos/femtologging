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
        let budget = shard.budget();
        let queued = QueuedRecord {
            record: FemtoLogRecord::new("pool", FemtoLevel::Info, "dispatched"),
            handlers: vec![handler.clone() as Arc<dyn FemtoHandlerTrait>],
            #[cfg(feature = "python")]
            context: None,
        };
        assert!(
            shard.admit(Box::new(queued), &budget).is_ok(),
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

    let budget = shard.budget();
    std::thread::sleep(idle_tick * 2);

    let handler = Arc::new(CollectingHandler::new());
    let queued = QueuedRecord {
        record: FemtoLogRecord::new("idle", FemtoLevel::Info, "after idling"),
        handlers: vec![handler.clone() as Arc<dyn FemtoHandlerTrait>],
        #[cfg(feature = "python")]
        context: None,
    };
    assert!(
        shard.admit(Box::new(queued), &budget).is_ok(),
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
    let budget = shard.budget();
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
        shard.admit(Box::new(queued("held")), &budget).is_ok(),
        "the first record fits"
    );
    started_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("the worker should start handling the first record");

    let refused = shard.admit(Box::new(queued("refused")), &budget);
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

/// A handler that blocks until released, reporting when it has started.
///
/// Shared by the starvation tests below: one logger's handler is held inside
/// `handle`, which is what makes the difference between a shard that is merely
/// shared and one where one logger can consume every permit.
struct HeldHandler {
    started: std::sync::mpsc::Sender<()>,
    release: parking_lot::Mutex<std::sync::mpsc::Receiver<()>>,
}

impl FemtoHandlerTrait for HeldHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        let _ = self.started.send(());
        let _ = self.release.lock().recv();
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// One logger at its in-flight bound must not starve another on its shard.
///
/// The shard-wide permit pool alone does not give this: a logger whose handler
/// never returns takes every permit the shard holds, and the other loggers
/// assigned to that shard are then refused records they could have handled.
/// The per-logger budget is what makes sharing a shard mean sharing, and the
/// assertion is that a *different* logger still gets its record delivered
/// while the first is stuck.
#[test]
fn one_stuck_logger_does_not_starve_another_on_its_shard() {
    // A shard deep enough for the other logger's record to still fit after the
    // stuck logger spends its allowance. The per-logger limit is set to one so
    // a two-deep shard can demonstrate the separation: the stuck logger takes
    // its one permit, the shard keeps one, and the other logger is served from
    // the pool the stuck logger could not drain.
    let shard = Shard::start(0, 2).expect("a test shard should start");
    let stuck_budget = shard.budget_with_limit_for_test(1);
    let other_budget = shard.budget();

    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let stuck_handler = Arc::new(HeldHandler {
        started: started_tx,
        release: parking_lot::Mutex::new(release_rx),
    }) as Arc<dyn FemtoHandlerTrait>;

    let queued = |name: &'static str, message: &str, handler: &Arc<dyn FemtoHandlerTrait>| {
        Box::new(QueuedRecord {
            record: FemtoLogRecord::new(name, FemtoLevel::Info, message),
            handlers: vec![Arc::clone(handler)],
        })
    };

    // Spend the stuck logger's whole allowance. Its handler never returns, so
    // the permit it took is not coming back while the test runs.
    assert!(
        shard
            .admit(queued("stuck", "first", &stuck_handler), &stuck_budget)
            .is_ok(),
        "the first record fits"
    );
    started_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("the worker should start handling the first record");
    assert!(
        shard
            .admit(queued("stuck", "second", &stuck_handler), &stuck_budget)
            .is_err(),
        "a logger at its own in-flight allowance must be refused even though \
         the shard still has permits"
    );

    // The other logger's record is refused only if one logger could take the
    // whole shard. It must be accepted: the stuck logger has spent its own
    // allowance, not every logger's.
    let collector = Arc::new(CollectingHandler::new());
    let other_handler = Arc::clone(&collector) as Arc<dyn FemtoHandlerTrait>;
    assert!(
        shard
            .admit(queued("other", "delivered", &other_handler), &other_budget)
            .is_ok(),
        "a logger must not be refused because a different logger filled its own \
         in-flight budget"
    );

    // Let the stuck handler finish so the test does not leak a blocked worker.
    release_tx
        .send(())
        .expect("the worker should still be waiting for its first record");
    assert!(
        shard.wait_for_drain(),
        "the shard should answer a barrier once its handlers return"
    );
    assert_eq!(
        collected_messages(&collector),
        vec!["delivered"],
        "the other logger's record must be handled by the same shard"
    );
}
