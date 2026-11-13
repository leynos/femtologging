//! Tests for rate-limited warning emission.

use super::*;
use proptest::prelude::*;
use rstest::*;
use std::sync::atomic::AtomicU64;

/// Atomic clock fixture that lets warning intervals advance deterministically.
struct FakeClock {
    /// Current fake time in milliseconds.
    now: AtomicU64,
}

impl Default for FakeClock {
    fn default() -> Self {
        Self {
            now: AtomicU64::new(0),
        }
    }
}

impl Clock for FakeClock {
    fn now_millis(&self) -> u64 {
        self.now.load(Ordering::Relaxed)
    }
}

impl FakeClock {
    fn advance(&self, ms: u64) {
        self.now.fetch_add(ms, Ordering::Relaxed);
    }
}

#[fixture]
fn clock() -> Arc<FakeClock> {
    Arc::new(FakeClock::default())
}

#[fixture]
fn warner() -> (RateLimitedWarner, Arc<FakeClock>) {
    let clock = Arc::new(FakeClock::default());
    (
        RateLimitedWarner::with_clock(Duration::from_secs(1), clock.clone() as Arc<dyn Clock>),
        clock,
    )
}

#[fixture]
fn warnings() -> Vec<u64> {
    Vec::new()
}

#[rstest]
#[case(1)]
#[case(3)]
fn accumulates_drops_and_emits_warning(
    warner: (RateLimitedWarner, Arc<FakeClock>),
    mut warnings: Vec<u64>,
    #[case] drop_count: u64,
) {
    let (warner, _clock) = warner;
    for _ in 0..drop_count {
        warner.record_drop();
    }
    warner.warn_if_due(|c| warnings.push(c));
    assert_eq!(warnings, vec![drop_count]);
}

#[rstest]
fn rate_limits_subsequent_warnings(
    mut warnings: Vec<u64>,
    warner: (RateLimitedWarner, Arc<FakeClock>),
) {
    let (warner, clock) = warner;
    warner.record_drop();
    warner.warn_if_due(|c| warnings.push(c));
    // Suppressed by the interval, but not forgotten: the count stays
    // pending and is reported with the next warning.
    warner.record_drop();
    warner.warn_if_due(|c| warnings.push(c));
    assert_eq!(warnings, vec![1]);
    clock.advance(1000);
    warner.record_drop();
    warner.warn_if_due(|c| warnings.push(c));
    assert_eq!(
        warnings,
        vec![1, 2],
        "the record suppressed with the interval must be counted by the \
         warning that follows it, not discarded"
    );
}

/// A warning a handler could not enqueue is retried, not lost.
#[rstest]
fn restores_a_warning_that_could_not_be_enqueued(
    warner: (RateLimitedWarner, Arc<FakeClock>),
    mut warnings: Vec<u64>,
) {
    let (warner, _clock) = warner;
    warner.record_drop();
    let due = warner
        .take_due()
        .expect("the first drop should be due for a warning");
    assert_eq!(due, 1);

    // The handler failed to enqueue its flush, so the count returns to the
    // pending state and the next attempt reports it even though the
    // interval has not advanced.
    warner.restore_drops(due);
    warner.warn_if_due(|count| warnings.push(count));
    assert_eq!(warnings, vec![1]);
}

/// Repeated failed enqueues accumulate until one succeeds.
#[rstest]
fn accumulates_warnings_restored_by_repeated_failures(
    warner: (RateLimitedWarner, Arc<FakeClock>),
    mut warnings: Vec<u64>,
) {
    let (warner, _clock) = warner;
    for _ in 0..2 {
        warner.record_drop();
    }
    let first = warner.take_due().expect("the first warning should be due");
    warner.restore_drops(first);
    let second = warner
        .take_due()
        .expect("the restored warning should be retried");
    warner.restore_drops(second);

    warner.warn_if_due(|count| warnings.push(count));
    assert_eq!(
        warnings,
        vec![2],
        "both failed attempts must be reported exactly once"
    );
}

#[rstest]
fn flush_emits_pending_warning(
    warner: (RateLimitedWarner, Arc<FakeClock>),
    mut warnings: Vec<u64>,
) {
    let (warner, _) = warner;
    warner.record_drop();
    warner.flush(|c| warnings.push(c));
    assert_eq!(warnings, vec![1]);
}

/// A count restored after a failed enqueue is reported by the next flush.
#[rstest]
fn flush_reports_restored_counts(
    warner: (RateLimitedWarner, Arc<FakeClock>),
    mut warnings: Vec<u64>,
) {
    let (warner, _) = warner;
    warner.record_drop();
    let due = warner.take_due().expect("the first drop should be due");
    warner.restore_drops(due);
    warner.flush(|count| warnings.push(count));
    assert_eq!(warnings, vec![1]);
}

/// A flush restarts the interval rather than only draining the drops.
#[rstest]
fn flush_restarts_the_warning_interval(
    warner: (RateLimitedWarner, Arc<FakeClock>),
    mut warnings: Vec<u64>,
) {
    let (warner, clock) = warner;
    warner.record_drop();
    warner.flush(|c| warnings.push(c));
    assert_eq!(warnings, vec![1]);

    clock.advance(500);
    warner.record_drop();
    warner.warn_if_due(|count| warnings.push(count));
    assert_eq!(
        warnings,
        vec![1],
        "a flush restarts the interval, so a warning half an interval \
         later must be suppressed"
    );

    clock.advance(500);
    warner.record_drop();
    warner.warn_if_due(|count| warnings.push(count));
    assert_eq!(
        warnings,
        vec![1, 2],
        "the record suppressed after the flush must be counted by the \
         next warning"
    );
}

#[rstest]
fn no_warning_when_no_drops(warner: (RateLimitedWarner, Arc<FakeClock>), mut warnings: Vec<u64>) {
    let (warner, _) = warner;
    warner.warn_if_due(|c| warnings.push(c));
    assert!(warnings.is_empty());
}

#[rstest]
fn flush_with_no_drops_does_nothing(
    warner: (RateLimitedWarner, Arc<FakeClock>),
    mut warnings: Vec<u64>,
) {
    let (warner, _) = warner;
    warner.flush(|c| warnings.push(c));
    assert!(warnings.is_empty());
}

/// One generated step in a drop-warning retry sequence.
#[derive(Clone, Debug)]
enum WarnerOp {
    /// Record drops, as a full queue or failed delivery would.
    Record(usize),
    /// Advance the shared clock by a number of milliseconds.
    Advance(u64),
    /// Take a due count without invoking a callback.
    TakeDue,
    /// Emit through the ordinary warning path.
    WarnIfDue,
    /// Return the outstanding taken count to the retry state.
    RestoreLast,
}

fn arb_warner_op() -> impl Strategy<Value = WarnerOp> {
    prop_oneof![
        (0usize..=3).prop_map(WarnerOp::Record),
        (0u64..=2_000).prop_map(WarnerOp::Advance),
        Just(WarnerOp::TakeDue),
        Just(WarnerOp::WarnIfDue),
        Just(WarnerOp::RestoreLast),
    ]
}

proptest! {
    /// Every recorded drop stays pending, in flight, or emitted exactly once.
    ///
    /// `take_due` models a flush handing its count to a worker queue: the
    /// count leaves the pending state and is either emitted or restored
    /// after a failed enqueue. A count taken twice without a restore is
    /// accounted as emitted, because production has already transferred
    /// it. Repeated restores must not duplicate the same drop.
    #[test]
    fn every_drop_is_pending_in_flight_or_emitted_once(
        ops in prop::collection::vec(arb_warner_op(), 0..60),
    ) {
        let clock = Arc::new(FakeClock::default());
        let warner = RateLimitedWarner::with_clock(
            Duration::from_secs(1),
            clock.clone() as Arc<dyn Clock>,
        );
        let mut recorded = 0_u64;
        let mut emitted = 0_u64;
        let mut in_flight: Option<u64> = None;

        for op in ops {
            match op {
                WarnerOp::Record(count) => {
                    for _ in 0..count {
                        warner.record_drop();
                    }
                    recorded += count as u64;
                }
                WarnerOp::Advance(ms) => clock.advance(ms),
                WarnerOp::TakeDue => {
                    if let Some(previous) = in_flight.take() {
                        emitted += previous;
                    }
                    in_flight = warner.take_due();
                }
                WarnerOp::WarnIfDue => {
                    warner.warn_if_due(|count| emitted += count);
                }
                WarnerOp::RestoreLast => {
                    if let Some(count) = in_flight.take() {
                        warner.restore_drops(count);
                    }
                }
            }

            let pending = warner.dropped.load(Ordering::Relaxed)
                + warner.retry_drops.load(Ordering::Relaxed);
            prop_assert_eq!(
                recorded,
                pending + emitted + in_flight.unwrap_or(0),
                "each drop must be pending, in flight, or emitted once",
            );
            prop_assert!(emitted <= recorded);
        }
    }
}

/// A flush whose command never enters a saturated queue must hand its due
/// drop count back, so the next warning still reports those records.
#[test]
fn failed_flush_enqueue_restores_the_due_drop_count() {
    let clock = Arc::new(FakeClock::default());
    let warner = RateLimitedWarner::with_clock(Duration::from_secs(5), clock);
    for _ in 0..3 {
        warner.record_drop();
    }
    // A fresh warner reports its first count immediately, so the flush below
    // takes all three drops and must put them back.
    let (tx, _rx) = crate::sync::bounded::<()>(1);
    tx.try_send(())
        .expect("the queue should accept its one item");

    let start = Instant::now();
    let deadline = start + Duration::from_millis(50);
    let mut now = || start;
    let outcome: Result<(), crate::handler::QueueFlushFailure> =
        warner.flush_with_pending_drops(deadline, &mut now, |pending_drops, budget| {
            assert_eq!(
                pending_drops,
                Some(3),
                "the flush must carry the count it took"
            );
            tx.send_timeout((), budget).ok()?;
            Some(|_: Duration| Some(()))
        });

    assert_eq!(
        outcome,
        Err(crate::handler::QueueFlushFailure::Enqueue),
        "a full queue refuses the flush command"
    );
    assert_eq!(
        warner.take_due(),
        Some(3),
        "the restored count must be the next warning's figure"
    );
}
