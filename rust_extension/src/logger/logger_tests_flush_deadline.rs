//! Tests that handler flushes share the logger barrier's absolute deadline.

use std::any::Any;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::handler::{FemtoHandlerTrait, HandlerError, capped_deadline, remaining};
use crate::log_record::FemtoLogRecord;
use crate::sync::{Mutex, Receiver, Sender, bounded};

use super::logger_tests_flush::flush_bound;
use crate::logger::{FemtoLogger, Shard};

use super::super::LOGGER_FLUSH_TIMEOUT_MS;

/// Build an isolated logger so another test's shard work cannot spend its budget.
fn test_logger() -> FemtoLogger {
    let shard = Box::leak(Box::new(
        Shard::start(0, 4).expect("a test shard should start"),
    ));
    FemtoLogger::assembled_for_test("flush".to_string(), shard)
}

/// A handler that records the wait its flush was given, and optionally spends it.
///
/// It takes the wait the barrier hands it, capped by a budget of its own, and
/// records what that came to before spending any of it. The record is what
/// makes the barrier's bookkeeping visible: a barrier that passes the same
/// deadline to every handler shows a shrinking wait across them, while one
/// that computes a fresh budget per handler — or hands over none at all —
/// shows each handler getting the full timeout.
struct WaitingFlushHandler {
    /// Held, never sent to, so the channel stays connected: a `recv_timeout`
    /// on a channel whose sender has been dropped returns `Disconnected` at
    /// once instead of waiting, which would make every wait below vacuous.
    _release: Sender<()>,
    /// Never sent to, so a waiting flush spends the whole wait it is given.
    release: Receiver<()>,
    /// The handler's own upper bound on one flush.
    budget: Duration,
    /// How long this handler spends on its own work before confirming.
    ///
    /// `None` makes the handler a pure recorder: it never confirms, so its
    /// flush spends the whole wait it was given.
    spend: Option<Duration>,
    /// The wait each flush was actually given.
    waited: Mutex<Vec<Duration>>,
}

impl WaitingFlushHandler {
    /// A handler willing to spend `budget` on its own flush, and never confirming.
    fn with_budget(budget: Duration) -> Self {
        Self::new(budget, None)
    }

    /// A handler that spends `spend` of its wait, then reports success.
    fn spending(spend: Duration) -> Self {
        Self::new(Duration::from_millis(LOGGER_FLUSH_TIMEOUT_MS), Some(spend))
    }

    fn new(budget: Duration, spend: Option<Duration>) -> Self {
        let (release_tx, release_rx) = bounded(1);
        Self {
            _release: release_tx,
            release: release_rx,
            budget,
            spend,
            waited: Mutex::new(Vec::new()),
        }
    }

    /// The waits handed to the flushes so far.
    fn waits(&self) -> Vec<Duration> {
        self.waited.lock().clone()
    }
}

impl FemtoHandlerTrait for WaitingFlushHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn flush_until(&self, deadline: Instant) -> bool {
        let wait = remaining(capped_deadline(deadline, self.budget));
        self.waited.lock().push(wait);

        let Some(spend) = self.spend else {
            // A handler with no work of its own never confirms, so it waits
            // out the whole wait it was given and the barrier sees a flush
            // that did not finish in time.
            let _ = self.release.recv_timeout(wait);
            return false;
        };

        // This handler has work to do. Nothing is ever sent on `release`, so
        // the wait ends at whichever is shorter: the work itself, or however
        // much of the caller's deadline is left for it. Either way the handler
        // confirms: the point of `spend` is the wall-clock it consumes inside
        // the barrier, and a comparison against the clock would only add a
        // way for a loaded host to report a failure the test does not mean.
        let _ = self.release.recv_timeout(wait.min(spend));
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[test]
fn flush_handlers_hands_each_handler_the_barriers_remaining_budget() {
    // A handler's flush never confirms here, so the flush runs until whatever
    // bound it was given. The handler is willing to spend three times the
    // logger's whole timeout on itself: a barrier that handed it a fresh
    // budget of its own -- or none at all -- would take that long, and both
    // the recorded wait and the elapsed time below would show it.
    let handler = Arc::new(WaitingFlushHandler::with_budget(Duration::from_millis(
        3 * LOGGER_FLUSH_TIMEOUT_MS,
    )));
    let logger = test_logger();
    logger
        .add_handler(Arc::clone(&handler) as Arc<dyn FemtoHandlerTrait>)
        .expect("the deadline-aware handler should register");

    let started_at = Instant::now();
    let flushed = logger.flush_handlers();
    let elapsed = started_at.elapsed();

    assert!(
        !flushed,
        "a handler whose flush never confirms must not report success"
    );

    let waits = handler.waits();
    assert_eq!(
        waits.len(),
        1,
        "the barrier should flush each handler exactly once"
    );
    assert!(
        waits[0] <= Duration::from_millis(LOGGER_FLUSH_TIMEOUT_MS),
        "the handler's flush was given {:?}; the barrier's timeout is \
         {LOGGER_FLUSH_TIMEOUT_MS} ms, so the wait it hands out cannot exceed it",
        waits[0]
    );
    assert!(
        elapsed < flush_bound(),
        "flush waited {elapsed:?} on a handler's own flush budget; the \
         timeout is {LOGGER_FLUSH_TIMEOUT_MS} ms and the bound is {:?}",
        flush_bound()
    );
}

#[test]
fn flush_handlers_shares_one_deadline_across_every_handler() {
    // The first handler spends a measurable part of the barrier's budget, so
    // the second one must be offered the remainder rather than a fresh copy.
    // Reissuing the full budget per handler is exactly the defect the barrier
    // exists to prevent: it would let N slow handlers spend the advertised
    // timeout N times over.
    let spend = Duration::from_millis(200);
    let first = Arc::new(WaitingFlushHandler::spending(spend));
    let second = Arc::new(WaitingFlushHandler::spending(Duration::ZERO));
    let logger = test_logger();
    logger
        .add_handler(Arc::clone(&first) as Arc<dyn FemtoHandlerTrait>)
        .expect("the first deadline-aware handler should register");
    logger
        .add_handler(Arc::clone(&second) as Arc<dyn FemtoHandlerTrait>)
        .expect("the second deadline-aware handler should register");

    assert!(logger.flush_handlers(), "both handlers report success");

    let waits = second.waits();
    assert_eq!(
        waits.len(),
        1,
        "the second handler should flush exactly once"
    );
    let full_budget = Duration::from_millis(LOGGER_FLUSH_TIMEOUT_MS);
    // The second handler's wait is the deadline minus everything already
    // spent, and the first handler spent `spend` on its own flush. Timers can
    // wake a hair early, so the comparison carries a slack — still far below
    // `spend`, and so still far too tight for a barrier that reissued the
    // whole timeout per handler, which would report `full_budget` here.
    let slack = Duration::from_millis(20);
    assert!(
        waits[0] <= full_budget - spend + slack,
        "the second handler was offered {:?}; the {spend:?} the first \
         handler spent should have come out of the barrier's {} ms budget",
        waits[0],
        LOGGER_FLUSH_TIMEOUT_MS
    );
    assert!(
        first.waits()[0] <= full_budget,
        "the barrier's own budget must reach the first handler too"
    );
}
