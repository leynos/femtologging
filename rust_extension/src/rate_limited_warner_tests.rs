//! Tests for rate-limited warning emission.
//!
//! Split from `rate_limited_warner.rs` to keep both files within the
//! repository's 400-line limit. These tests reach the warner's private state
//! through the same module boundary the production code uses, so they need no
//! test-only widening of the public API beyond [`pending_drops`].
//!
//! [`pending_drops`]: RateLimitedWarner::pending_drops

use super::*;
use rstest::*;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Barrier, Mutex, mpsc};
use std::thread;

/// A test clock that advances only when the test requests it.
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
fn warner(clock: Arc<FakeClock>) -> (RateLimitedWarner, Arc<FakeClock>) {
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

/// Scenario: a drop is recorded inside the interval after a warning has been
/// emitted, and the warning it would have produced is suppressed.
///
/// Invariant: the suppressed attempt leaves its count pending, so the next due
/// warning reports both drops rather than only the one recorded after the
/// suppression.
#[rstest]
fn rate_limits_subsequent_warnings(
    mut warnings: Vec<u64>,
    warner: (RateLimitedWarner, Arc<FakeClock>),
) {
    let (warner, clock) = warner;
    warner.record_drop();
    warner.warn_if_due(|c| warnings.push(c));
    warner.record_drop();
    warner.warn_if_due(|c| warnings.push(c));
    assert_eq!(warnings, vec![1]);
    clock.advance(1000);
    warner.record_drop();
    warner.warn_if_due(|c| warnings.push(c));
    assert_eq!(warnings, vec![1, 2]);
}

/// Scenario: three drops are recorded inside one suppression window, and each
/// is offered to `warn_if_due()` in turn.
///
/// Invariant: the next due warning reports all three plus whatever was already
/// pending. This is the regression the module was fixed for: the old
/// suppressed-path reset discarded the three, leaving only the fourth drop in
/// the count.
#[rstest]
fn suppressed_attempts_carry_drops_forward(
    mut warnings: Vec<u64>,
    warner: (RateLimitedWarner, Arc<FakeClock>),
) {
    let (warner, clock) = warner;
    warner.record_drop();
    warner.warn_if_due(|c| warnings.push(c));

    for _ in 0..3 {
        warner.record_drop();
        warner.warn_if_due(|c| warnings.push(c));
    }
    assert_eq!(warnings, vec![1], "the interval should suppress all three");
    assert_eq!(warner.pending_drops(), 3, "and preserve all three");

    clock.advance(1000);
    warner.record_drop();
    warner.warn_if_due(|c| warnings.push(c));
    assert_eq!(warnings, vec![1, 4]);
}

/// Scenario: `flush()` is called inside the suppression window.
///
/// Invariant: it reports the pending drops at once and consumes them, so a
/// second flush is silent. It does not read the clock or consume the interval,
/// so a further drop inside the same window is still suppressed.
#[rstest]
fn flush_reports_inside_the_suppression_window(
    mut warnings: Vec<u64>,
    warner: (RateLimitedWarner, Arc<FakeClock>),
) {
    let (warner, clock) = warner;
    warner.record_drop();
    warner.warn_if_due(|c| warnings.push(c));
    warner.record_drop();
    warner.record_drop();

    warner.flush(|c| warnings.push(c));
    assert_eq!(warnings, vec![1, 2]);
    warner.flush(|c| warnings.push(c));
    assert_eq!(warnings, vec![1, 2], "a second flush has nothing to report");

    warner.record_drop();
    warner.warn_if_due(|c| warnings.push(c));
    assert_eq!(
        warnings,
        vec![1, 2],
        "flushing did not consume the interval"
    );

    clock.advance(1000);
    warner.record_drop();
    warner.warn_if_due(|c| warnings.push(c));
    assert_eq!(warnings, vec![1, 2, 2]);
}

/// Scenario: `warn_if_due()` is called when nothing has been dropped, and then
/// a drop arrives, with the clock already past the interval either way.
///
/// Invariant: the empty call does not start an interval, so the drop warns
/// immediately. A poller with nothing to report must not hold the first real
/// warning back for a whole interval.
#[rstest]
#[case(1)]
#[case(2)]
fn empty_call_does_not_start_the_window(
    warner: (RateLimitedWarner, Arc<FakeClock>),
    mut warnings: Vec<u64>,
    #[case] intervals_elapsed: u64,
) {
    let (warner, clock) = warner;
    clock.advance(intervals_elapsed * 1000);
    warner.warn_if_due(|c| warnings.push(c));
    assert!(warnings.is_empty());

    warner.record_drop();
    warner.warn_if_due(|c| warnings.push(c));
    assert_eq!(warnings, vec![1]);
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

/// Only one CAS accepts callers that observed the same expired timestamp.
#[test]
fn compare_exchange_rejects_the_other_same_timestamp_claims() {
    const CALLERS: usize = 8;

    let last_warn = Arc::new(AtomicU64::new(0));
    let observed_prev = last_warn.load(Ordering::Relaxed);
    let start = Arc::new(Barrier::new(CALLERS));
    let successful_claims = AtomicU64::new(0);

    thread::scope(|scope| {
        for _ in 0..CALLERS {
            let last_warn = Arc::clone(&last_warn);
            let start = Arc::clone(&start);
            let successful_claims = &successful_claims;
            scope.spawn(move || {
                start.wait();
                if try_claim_interval(&last_warn, observed_prev, 1000) {
                    successful_claims.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
    });

    assert_eq!(successful_claims.load(Ordering::Relaxed), 1);
    assert_eq!(last_warn.load(Ordering::Relaxed), 1000);
}

/// A slow callback does not hold up another warning attempt or flush.
#[test]
fn callback_runs_after_releasing_the_drain_lock() {
    let warner = Arc::new(RateLimitedWarner::new(Duration::from_secs(3600)));
    warner.record_drop();

    let callback_entered = Arc::new(Barrier::new(2));
    let callback_resume = Arc::new(Barrier::new(2));
    let second_attempt_started = Arc::new(Barrier::new(2));
    let warning_counts = Arc::new(Mutex::new(Vec::new()));
    let (second_done_tx, second_done_rx) = mpsc::channel();

    let second_attempt_finished_while_callback_paused = thread::scope(|scope| {
        let first_warner = Arc::clone(&warner);
        let first_callback_entered = Arc::clone(&callback_entered);
        let first_callback_resume = Arc::clone(&callback_resume);
        let first_counts = Arc::clone(&warning_counts);
        scope.spawn(move || {
            first_warner.warn_if_due(|count| {
                first_counts
                    .lock()
                    .expect("warning counts lock")
                    .push(count);
                first_callback_entered.wait();
                first_callback_resume.wait();
            });
        });
        callback_entered.wait();

        let second_warner = Arc::clone(&warner);
        let worker_attempt_started = Arc::clone(&second_attempt_started);
        let second_counts = Arc::clone(&warning_counts);
        scope.spawn(move || {
            second_warner.record_drop();
            worker_attempt_started.wait();
            second_warner.warn_if_due(|count| {
                second_counts
                    .lock()
                    .expect("warning counts lock")
                    .push(count);
            });
            second_done_tx.send(()).expect("second attempt receiver");
        });
        second_attempt_started.wait();
        let finished_early = second_done_rx.recv_timeout(Duration::from_secs(1)).is_ok();
        callback_resume.wait();
        finished_early
    });

    assert!(
        second_attempt_finished_while_callback_paused,
        "warning callbacks must run outside the drain mutex"
    );
    assert_eq!(
        *warning_counts.lock().expect("warning counts lock"),
        vec![1]
    );
    assert_eq!(warner.pending_drops(), 1);
}

/// Scenario: threads record, warn, flush and move the clock concurrently.
///
/// Invariant: conservation. Every recorded drop is reported exactly once, so
/// the callback counts sum to the number of `record_drop` calls, and no
/// callback is handed an empty batch.
#[rstest]
#[case::two_threads(2, 60)]
#[case::four_threads(4, 40)]
fn concurrent_callers_conserve_every_drop(#[case] thread_count: usize, #[case] ops: usize) {
    let clock = Arc::new(FakeClock::default());
    // A one-millisecond interval, so the clock advance on thread 0 crosses the
    // boundary and the run mixes due and suppressed calls.
    let warner = Arc::new(RateLimitedWarner::with_clock(
        Duration::from_millis(1),
        Arc::clone(&clock) as Arc<dyn Clock>,
    ));
    let recorded = AtomicU64::new(0);
    let reported = AtomicU64::new(0);
    let saw_empty_batch = AtomicBool::new(false);
    let start = Barrier::new(thread_count);

    thread::scope(|scope| {
        for index in 0..thread_count {
            let warner = Arc::clone(&warner);
            let clock = Arc::clone(&clock);
            let recorded = &recorded;
            let reported = &reported;
            let saw_empty_batch = &saw_empty_batch;
            let start = &start;
            scope.spawn(move || {
                start.wait();
                let mut warn = |count: u64| {
                    if count == 0 {
                        saw_empty_batch.store(true, Ordering::Relaxed);
                    }
                    reported.fetch_add(count, Ordering::Relaxed);
                };
                for step in 0..ops {
                    warner.record_drop();
                    recorded.fetch_add(1, Ordering::Relaxed);
                    match step % 3 {
                        0 => warner.warn_if_due(&mut warn),
                        1 if index == 0 => clock.advance(1),
                        _ => warner.flush(&mut warn),
                    }
                }
            });
        }
    });

    let mut tail = Vec::new();
    warner.flush(|count| tail.push(count));
    let reported = reported.load(Ordering::Relaxed) + tail.iter().sum::<u64>();
    let recorded = recorded.load(Ordering::Relaxed);

    assert!(
        !saw_empty_batch.load(Ordering::Relaxed),
        "no callback should be handed an empty batch"
    );
    assert_eq!(
        warner.pending_drops(),
        0,
        "the final flush should leave nothing pending"
    );
    assert_eq!(
        reported, recorded,
        "every recorded drop should be reported exactly once"
    );
}
