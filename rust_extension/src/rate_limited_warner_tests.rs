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
use std::sync::Barrier;
use std::sync::atomic::AtomicU64;
use std::thread;

/// A test clock that advances only when the test requests it.
///
/// Shared with the concurrency tests, which mix clock advances into racing
/// warn and flush calls.
pub(super) struct FakeClock {
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
    /// Move the fake clock forward by `ms` milliseconds.
    pub(super) fn advance(&self, ms: u64) {
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
