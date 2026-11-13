//! Bounded property tests for the dispatch shard's flush handshake.
//!
//! The handshake has four observable stages — marker admission, snapshot
//! transfer, worker start, and acknowledgement — and one ordering promise: the
//! handlers a flush covers have already received every record admitted ahead of
//! its marker. Those properties hold for any number of handlers, any mix of
//! confirming and failing ones, and either rejection path, so they are
//! generated rather than enumerated. Each case runs against a real shard with a
//! budget no generated case can exhaust, so the assertions describe the
//! protocol rather than the host's scheduling.

use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

use proptest::prelude::*;

use crate::handler::{FemtoHandlerTrait, FlushContext, HandlerError};
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;
use crate::monotonic_clock::system_clock;
use crate::sync::{Mutex, bounded};

use crate::logger::flush_clock::FlushDeadline;
use crate::logger::flush_diagnostics::FlushFailureStage;
use crate::logger::{LoggerBudget, QueuedRecord, Shard};

/// A barrier budget longer than any generated case can spend.
const GENEROUS_BUDGET: Duration = Duration::from_secs(2);

/// Handler that records the dispatch and flush calls it receives, in order.
struct HandshakeHandler {
    /// Whether this handler confirms the flush it is given.
    confirms: bool,
    /// Worker-side calls in arrival order: each record message, then `flush`.
    events: Mutex<Vec<String>>,
}

impl HandshakeHandler {
    /// Build a handler that confirms or refuses its flush.
    fn new(confirms: bool) -> Self {
        Self {
            confirms,
            events: Mutex::new(Vec::new()),
        }
    }

    /// The calls this handler has observed so far.
    fn events(&self) -> Vec<String> {
        self.events.lock().clone()
    }
}

impl FemtoHandlerTrait for HandshakeHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        self.events.lock().push(record.message().to_owned());
        Ok(())
    }

    fn flush_until(&self, _context: &FlushContext) -> bool {
        self.events.lock().push("flush".to_owned());
        self.confirms
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Box a record carrying the same handler snapshot as the flush under test.
fn queued_record(message: &str, handlers: &[Arc<dyn FemtoHandlerTrait>]) -> Box<QueuedRecord> {
    Box::new(QueuedRecord {
        record: FemtoLogRecord::new("flush-handshake", FemtoLevel::Info, message),
        handlers: handlers.to_vec(),
        #[cfg(feature = "python")]
        context: None,
    })
}

/// Start a shard with room for the records the generated case admits.
fn shard_with_room(record_count: usize) -> (Shard, Arc<LoggerBudget>) {
    let depth = record_count + 2;
    let shard = Shard::start(0, depth).expect("a test shard should start");
    let budget = shard.budget_with_limit_for_test(depth);
    (shard, budget)
}

/// Run one flush whose snapshot producer always succeeds.
fn flush(
    shard: &Shard,
    snapshot: Vec<Arc<dyn FemtoHandlerTrait>>,
) -> Result<(), FlushFailureStage> {
    let timing = FlushDeadline::with_clock(GENEROUS_BUDGET, system_clock());
    shard.flush_handlers_until(&timing, |sender| {
        sender
            .try_send(snapshot)
            .map_err(|_| FlushFailureStage::WorkerStartDisconnected)
    })
}

/// Wait for the shard's queue to drain, failing the generated case on timeout.
fn drain(shard: &Shard) -> Result<(), TestCaseError> {
    let timing = FlushDeadline::with_clock(GENEROUS_BUDGET, system_clock());
    shard.wait_for_drain_until(&timing).map_err(|stage| {
        TestCaseError::fail(format!(
            "the shard must still drain after a flush: {stage:?}"
        ))
    })
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 24,
        max_shrink_iters: 16,
        ..ProptestConfig::default()
    })]

    /// A flush confirms only when every handler does, always follows the
    /// records admitted ahead of its marker, and stops at the first handler
    /// that declines.
    #[test]
    fn a_flush_confirms_with_its_handlers_and_follows_prior_records(
        record_count in 0_usize..6,
        confirms in prop::collection::vec(any::<bool>(), 1..4),
    ) {
        let (shard, budget) = shard_with_room(record_count);
        let handlers = confirms
            .iter()
            .map(|confirms| Arc::new(HandshakeHandler::new(*confirms)))
            .collect::<Vec<_>>();
        let snapshot = handlers
            .iter()
            .map(|handler| Arc::clone(handler) as Arc<dyn FemtoHandlerTrait>)
            .collect::<Vec<_>>();

        for index in 0..record_count {
            shard
                .admit(queued_record(&format!("record-{index}"), &snapshot), &budget)
                .map_err(|_| TestCaseError::fail("the shard must admit records within its depth"))?;
        }

        // The sequence stops at the first handler that declines, so only the
        // handlers up to that one receive the flush callback.
        let first_decliner = confirms.iter().position(|confirms| !*confirms);
        let outcome = flush(&shard, snapshot.clone());
        prop_assert_eq!(outcome.is_ok(), first_decliner.is_none());
        if first_decliner.is_some() {
            prop_assert_eq!(outcome, Err(FlushFailureStage::HandlerFlushUnconfirmed));
        }

        // Every visited handler saw the records first and the flush last. An
        // empty record set still leaves the flush as the only call.
        let records = (0..record_count)
            .map(|index| format!("record-{index}"))
            .collect::<Vec<_>>();
        for (index, handler) in handlers.iter().enumerate() {
            let was_visited = first_decliner.is_none_or(|decliner| index <= decliner);
            let mut expected_events = records.clone();
            if was_visited {
                expected_events.push("flush".to_owned());
            }
            prop_assert_eq!(
                handler.events(),
                expected_events,
                "a flush must follow every record admitted ahead of its marker, \
                 and must not skip past a handler that declined",
            );
        }

        // Rejection of a handler's flush must not wedge the shard: a record
        // admitted afterwards is still dispatched.
        shard
            .admit(queued_record("after", &snapshot), &budget)
            .map_err(|_| TestCaseError::fail("the shard must admit a record after a flush"))?;
        drain(&shard)?;
        for handler in &handlers {
            let events = handler.events();
            prop_assert_eq!(
                events.last().map(String::as_str),
                Some("after"),
                "the worker must keep draining after a flush",
            );
        }
    }

    /// A flush rejected before the worker starts never reaches a handler, and
    /// leaves the shard usable.
    #[test]
    fn a_rejected_flush_never_reaches_a_handler(
        record_count in 0_usize..4,
        reject_with_snapshot_failure in any::<bool>(),
    ) {
        let (shard, budget) = shard_with_room(record_count);
        let handler = Arc::new(HandshakeHandler::new(true));
        let snapshot = vec![Arc::clone(&handler) as Arc<dyn FemtoHandlerTrait>];
        for index in 0..record_count {
            shard
                .admit(queued_record(&format!("record-{index}"), &snapshot), &budget)
                .map_err(|_| TestCaseError::fail("the shard must admit records within its depth"))?;
        }

        let outcome = if reject_with_snapshot_failure {
            // The registry read times out, so no snapshot is transferred and
            // the start gate is closed by dropping its sender.
            let timing = FlushDeadline::with_clock(GENEROUS_BUDGET, system_clock());
            shard.flush_handlers_until(&timing, |_sender| {
                Err(FlushFailureStage::HandlerSnapshotTimedOut)
            })
        } else {
            // A deadline that has already elapsed stops before the marker is
            // admitted, so the queue keeps only the records ahead of it.
            let timing = FlushDeadline::with_clock(Duration::ZERO, system_clock());
            shard.flush_handlers_until(&timing, |_sender| Ok(()))
        };
        let expected = if reject_with_snapshot_failure {
            FlushFailureStage::HandlerSnapshotTimedOut
        } else {
            FlushFailureStage::DeadlineExpiredBeforeMarker
        };
        prop_assert_eq!(outcome, Err(expected));

        // The records ahead of the marker belong to the worker, but the flush
        // callback itself must not have run.
        prop_assert!(
            !handler.events().iter().any(|event| event == "flush"),
            "a flush rejected before the worker starts must not run a callback",
        );

        // The rejection is local to the marker: the shard still takes a record
        // and a fresh flush confirms afterwards.
        shard
            .admit(queued_record("after", &snapshot), &budget)
            .map_err(|_| TestCaseError::fail("the shard must stay usable after a rejected flush"))?;
        drain(&shard)?;
        prop_assert_eq!(flush(&shard, snapshot.clone()), Ok(()));
        let events = handler.events();
        prop_assert_eq!(
            events.last().map(String::as_str),
            Some("flush"),
            "a confirmed flush must run after the rejected one",
        );
    }
}

/// The worker waits for the snapshot producer to return before it runs a
/// callback, which is what keeps handler callbacks outside the registry lock.
#[test]
fn a_handler_is_not_invoked_until_the_snapshot_is_handed_over() {
    let (shard, _budget) = shard_with_room(0);
    let handler = Arc::new(HandshakeHandler::new(true));
    let snapshot = vec![Arc::clone(&handler) as Arc<dyn FemtoHandlerTrait>];
    let (snapshot_sent_tx, snapshot_sent_rx) = bounded(1);
    let (release_prepare_tx, release_prepare_rx) = bounded(1);

    std::thread::scope(|scope| {
        let caller = scope.spawn(|| {
            let timing = FlushDeadline::with_clock(GENEROUS_BUDGET, system_clock());
            shard.flush_handlers_until(&timing, |sender| {
                let _ = sender.try_send(snapshot);
                let _ = snapshot_sent_tx.send(());
                // Stands in for the registry read lock: the worker must not act
                // on the snapshot while its producer is still holding it.
                let _ = release_prepare_rx.recv();
                Ok(())
            })
        });

        snapshot_sent_rx
            .recv_timeout(GENEROUS_BUDGET)
            .expect("the snapshot producer should run");
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            handler.events().is_empty(),
            "the worker must not run a callback while the snapshot producer holds the lock",
        );
        assert!(
            !caller.is_finished(),
            "the flush must stay inside the snapshot producer until it returns",
        );

        release_prepare_tx
            .send(())
            .expect("the snapshot producer should be released");
        assert_eq!(
            caller.join().expect("the flush caller should finish"),
            Ok(()),
            "the released flush should confirm",
        );
    });

    assert_eq!(handler.events(), vec!["flush".to_owned()]);
}
