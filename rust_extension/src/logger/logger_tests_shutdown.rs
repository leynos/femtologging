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
/// The worker blocks on its queue, so an idle shard is a blocked receiver, not
/// a polling loop. That is the property this pins: silence from the queue must
/// not end the worker. A worker that retired while idle leaves the *next*
/// flush waiting on a barrier nobody answers, which is a failure that appears
/// long after the shard fell silent and in a different call — the reason it is
/// worth an explicit test rather than relying on tests that log, flush and
/// finish without pausing.
///
/// The pause is short and fixed. It is not racing a bound; it only has to
/// outlast the worker's transition from "has not started" to "blocked on an
/// empty queue", which happens as soon as the pool is constructed.
#[test]
fn an_idle_worker_keeps_answering() {
    let dispatcher = pool_with(1);
    let shard = dispatcher.assign_shard();

    let budget = shard.budget();
    std::thread::sleep(std::time::Duration::from_millis(50));

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
         retired while idle leaves the flush waiting for nobody"
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

/// One logger at its in-flight bound must not take another's queue slot.
///
/// The shard-wide permit pool alone does not give this: a logger that keeps
/// logging would take every permit the shard holds, and the other loggers
/// assigned to that shard would then be refused records they could have
/// handled. The per-logger budget is what makes sharing a shard mean sharing.
///
/// The assertion is about *admission*, and deliberately not about delivery: a
/// shard runs its queue on one worker, so a handler that never returns stops
/// delivery for every logger on that shard until it does. This test releases
/// the stuck handler before it checks delivery, and asserts only that the other
/// logger's record was *accepted* while the first was stuck.
#[test]
fn one_stuck_logger_does_not_take_another_logger_s_slot() {
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
    // Delivery is asserted only now, after the stuck handler was released: a
    // shard has one worker, so nothing behind the stuck record could have been
    // handled any earlier. What the budget bought was the slot, not the
    // delivery.
    assert_eq!(
        collected_messages(&collector),
        vec!["delivered"],
        "the other logger's record must be handled by the same shard once the \
         stuck handler returns"
    );
}

/// A flush by a logger with nothing in flight costs the shard nothing.
///
/// Two things go wrong without the guard, and the second is a correctness
/// problem rather than a cost.
///
/// A barrier takes a queue slot but no permit, and it waits for the worker
/// behind it to answer. So a logger that has nothing to drain still pays the
/// barrier timeout against a shard that is merely busy, which is what a
/// `flush_all_handlers` sweep would pay once per registered logger. Worse, when
/// the worker is stuck the barrier stays queued, and queued barriers can fill
/// the queue while permits remain — `admit` then refuses a record the shard had
/// room for, and a legitimate record is dropped.
///
/// The shard here is two deep, matching two flushing loggers, so the barriers
/// alone can fill it. The stuck logger's record is in the worker's hands,
/// holding one of the two permits, so one permit stays free throughout.
#[test]
fn a_logger_with_nothing_in_flight_does_not_barrier_its_shard() {
    // Leaked deliberately: a pool shard is borrowed for `'static` by every
    // logger on it, and this test builds the pool it shares rather than using
    // the process-wide one.
    let shard: &'static Shard = Box::leak(Box::new(
        Shard::start(0, 2).expect("a test shard should start"),
    ));

    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let stuck_handler = Arc::new(HeldHandler {
        started: started_tx,
        release: parking_lot::Mutex::new(release_rx),
    }) as Arc<dyn FemtoHandlerTrait>;

    let stuck = FemtoLogger::assembled_for_test("stuck".to_string(), shard);
    stuck.add_handler(stuck_handler);
    stuck.log(FemtoLevel::Info, "occupies the worker");
    started_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("the worker should start handling the record");

    // Two loggers with nothing of their own in flight. Without the guard each
    // queues a barrier behind the stuck record and then waits out the barrier
    // timeout, so both report a failed flush even though neither had anything
    // to drain.
    let idle_one = FemtoLogger::assembled_for_test("idle-one".to_string(), shard);
    let idle_two = FemtoLogger::assembled_for_test("idle-two".to_string(), shard);
    assert!(
        idle_one.flush_handlers() && idle_two.flush_handlers(),
        "a flush with nothing in flight must report that it drained, without \
         waiting on a shard whose worker is busy with someone else's record"
    );

    // Those two barriers would by now have filled the two queue slots while one
    // permit was still free, so this record would be refused and counted as
    // dropped against a shard that had room to take it.
    let collector = Arc::new(CollectingHandler::new());
    let fresh = FemtoLogger::assembled_for_test("fresh".to_string(), shard);
    fresh.add_handler(Arc::clone(&collector) as Arc<dyn FemtoHandlerTrait>);
    fresh.log(FemtoLevel::Info, "still fits");
    assert_eq!(
        fresh.get_dropped(),
        0,
        "flushes from idle loggers must not fill the shard's queue and make a \
         record be dropped while permits remain"
    );

    // Let the stuck handler finish so the test does not leak a blocked worker.
    release_tx
        .send(())
        .expect("the worker should still be waiting for its first record");
    assert!(
        fresh.flush_handlers(),
        "the shard should answer once its handlers return"
    );
    assert_eq!(
        collected_messages(&collector),
        vec!["still fits"],
        "the record admitted after the idle flushes must still be delivered"
    );
}
