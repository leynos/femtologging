//! Tests for the clock samples and ownership handoff of a warning claim.
//!
//! Split from `rate_limited_warner_tests.rs` to keep every file within the
//! repository's 400-line limit. The cases here need a clock whose samples the
//! test scripts or interleaves, which the shared `FakeClock` cannot express;
//! they reach the warner's private state through the same module boundary the
//! production code uses.

use super::*;
use std::collections::VecDeque;
use std::sync::atomic::AtomicU64;
use std::sync::{Barrier, Mutex, OnceLock, mpsc};
use std::thread;

/// A clock that returns a scripted sequence of samples, then a fixed value.
struct ScriptedClock {
    /// Samples to return, in order.
    samples: Mutex<VecDeque<u64>>,
    /// Value every sample returns once the script is exhausted.
    then: u64,
}

impl ScriptedClock {
    /// Build a clock with `samples` then a constant `then`.
    fn new(samples: impl IntoIterator<Item = u64>, then: u64) -> Self {
        Self {
            samples: Mutex::new(samples.into_iter().collect()),
            then,
        }
    }
}

impl Clock for ScriptedClock {
    fn now_millis(&self) -> u64 {
        self.samples
            .lock()
            .expect("scripted clock samples")
            .pop_front()
            .unwrap_or(self.then)
    }
}

/// Scenario: a due caller samples the clock, is pre-empted, and only wins its
/// claim once the clock has moved a whole interval past that sample.
///
/// Invariant: the interval starts when the report is published, not when it was
/// sampled, so the next caller inside the following interval stays suppressed
/// instead of warning immediately after the first.
#[test]
fn a_delayed_claim_sample_cannot_start_the_interval_in_the_past() {
    // Sample zero is the due pre-check and sample one the claim; the
    // publication sample that follows reports the clock an interval later.
    let clock = Arc::new(ScriptedClock::new([0, 0, 1000], 1000));
    let warner =
        RateLimitedWarner::with_clock(Duration::from_secs(1), Arc::clone(&clock) as Arc<dyn Clock>);
    let mut warnings = Vec::new();

    warner.record_drop();
    warner.warn_if_due(|count| warnings.push(count));
    assert_eq!(warnings, vec![1]);
    assert_eq!(
        warner.last_warn.load(Ordering::Relaxed),
        1000,
        "the published interval start must be the later publication sample"
    );

    warner.record_drop();
    warner.warn_if_due(|count| warnings.push(count));
    assert_eq!(
        warnings,
        vec![1],
        "a delayed claim must not let the next caller warn inside its interval"
    );
    assert_eq!(warner.pending_drops(), 1);
}

/// A clock that lets another caller claim the interval inside its own sample.
struct StealingClaimClock {
    /// Timestamp every sample returns.
    now_ms: u64,
    /// Timestamp the concurrent claim publishes.
    claimed_elsewhere: u64,
    /// Samples taken so far, so the concurrent claim lands on ours.
    samples: AtomicU64,
    /// Warner the concurrent claim is made on, set once it exists.
    warner: OnceLock<Arc<RateLimitedWarner>>,
}

impl Clock for StealingClaimClock {
    fn now_millis(&self) -> u64 {
        // Sample zero is the due pre-check and sample one the claim's own. A
        // concurrent caller claims the interval during that second sample, so
        // this caller's recheck must refuse its claim.
        if self.samples.fetch_add(1, Ordering::Relaxed) == 1
            && let Some(warner) = self.warner.get()
        {
            warner
                .last_warn
                .store(self.claimed_elsewhere, Ordering::Relaxed);
        }
        self.now_ms
    }
}

/// Scenario: a due call drains its batch and then loses the interval to a
/// caller that claimed it in the meantime.
///
/// Invariant: the refused call returns the batch it took, so the drops stay
/// pending and are reported by the next due call instead of vanishing into a
/// warning that was never emitted.
#[test]
fn a_refused_claim_returns_the_batch() {
    let clock = Arc::new(StealingClaimClock {
        now_ms: 1000,
        claimed_elsewhere: 1000,
        samples: AtomicU64::new(0),
        warner: OnceLock::new(),
    });
    let warner = Arc::new(RateLimitedWarner::with_clock(
        Duration::from_secs(1),
        Arc::clone(&clock) as Arc<dyn Clock>,
    ));
    assert!(
        clock.warner.set(Arc::clone(&warner)).is_ok(),
        "the clock's warner is set once, before any sample"
    );
    warner.record_drop();

    let mut warnings = Vec::new();
    warner.warn_if_due(|count| warnings.push(count));
    assert!(
        warnings.is_empty(),
        "the refused caller must not report a batch the interval does not cover"
    );
    assert_eq!(
        warner.pending_drops(),
        1,
        "the refused caller must return the batch it drained"
    );

    warner.flush(|count| warnings.push(count));
    assert_eq!(warnings, vec![1], "the returned batch is still reportable");
}

/// A flush does not wait for a paused warning callback.
///
/// The callback only reaches its barriers if it runs after the warner has
/// finished with the counter and the interval; a callback that held either
/// would make the flush below wait out its timeout.
#[test]
fn flush_does_not_wait_for_a_paused_warning_callback() {
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
            // Flush rather than warn: the interval is open, so `warn_if_due`
            // would return at the suppression pre-check without ever testing
            // whether a paused callback still holds the warner.
            second_warner.flush(|count| {
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
        "a flush must not wait for a paused warning callback"
    );
    assert_eq!(
        *warning_counts.lock().expect("warning counts lock"),
        vec![1, 1],
        "the paused warning and the flush each report their own drop"
    );
    assert_eq!(warner.pending_drops(), 0);
}
