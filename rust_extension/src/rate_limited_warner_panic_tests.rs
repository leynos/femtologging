//! Tests for a clock sample that panics while a due call holds a batch.
//!
//! Split from `rate_limited_warner_claim_tests.rs` to keep every file within
//! the repository's 400-line limit. A drained batch leaves the counter before
//! the clock is sampled, so these cases pin what the warner does with it when
//! the sample unwinds instead of returning.

use super::*;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::AtomicU64;

/// A clock that panics instead of returning one selected sample.
struct PanickingClock {
    /// Timestamp every other sample returns.
    now_ms: u64,
    /// Zero-based index of the sample that panics.
    panic_on: u64,
    /// Samples taken so far.
    samples: AtomicU64,
}

impl Clock for PanickingClock {
    fn now_millis(&self) -> u64 {
        let sample = self.samples.fetch_add(1, Ordering::Relaxed);
        assert!(
            sample != self.panic_on,
            "clock sample {sample} panics on request"
        );
        self.now_ms
    }
}

/// Run a warning against a clock that panics on `panic_on` and report the
/// warner, the number of callback invocations and the published interval.
fn warning_with_panicking_clock(panic_on: u64) -> (RateLimitedWarner, u64, u64) {
    let clock = Arc::new(PanickingClock {
        now_ms: 1000,
        panic_on,
        samples: AtomicU64::new(0),
    });
    let warner =
        RateLimitedWarner::with_clock(Duration::from_secs(1), Arc::clone(&clock) as Arc<dyn Clock>);
    warner.record_drop();

    let reported = AtomicU64::new(0);
    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
        warner.warn_if_due(|_| {
            reported.fetch_add(1, Ordering::Relaxed);
        });
    }));

    assert!(
        outcome.is_err(),
        "the scripted clock sample must unwind the call"
    );
    let published = warner.last_warn.load(Ordering::Relaxed);
    (warner, reported.into_inner(), published)
}

/// Scenario: the clock panics while the claim samples it.
///
/// Invariant: the drained batch is restored, so a panicking clock cannot lose
/// drops that no callback was given, and no interval is published.
#[test]
fn a_panicking_claim_sample_leaves_the_batch_pending() {
    let (warner, reported, published) = warning_with_panicking_clock(1);

    assert_eq!(reported, 0, "an unwound call hands nothing to its callback");
    assert_eq!(
        warner.pending_drops(),
        1,
        "a panic before the claim must leave the batch pending"
    );
    assert_eq!(
        published,
        u64::MAX,
        "a claim that panicked must not publish an interval"
    );

    let mut warnings = Vec::new();
    warner.flush(|count| warnings.push(count));
    assert_eq!(warnings, vec![1], "the restored batch is still reportable");
}

/// Scenario: the clock panics while the report time is sampled, after the claim
/// has already been won.
///
/// Invariant: the batch is restored. The claim itself cannot be withdrawn -
/// this sample is the only thing that would move it forward - so the caller
/// that panicked keeps the interval closed, but it must not also keep the
/// count.
#[test]
fn a_panicking_report_time_sample_leaves_the_batch_pending() {
    let (warner, reported, published) = warning_with_panicking_clock(2);

    assert_eq!(reported, 0, "an unwound call hands nothing to its callback");
    assert_eq!(
        warner.pending_drops(),
        1,
        "a panic after the claim must still leave the batch pending"
    );
    assert_eq!(
        published, 1000,
        "the won claim stays published; only the batch is restored"
    );

    let mut warnings = Vec::new();
    warner.flush(|count| warnings.push(count));
    assert_eq!(
        warnings,
        vec![1],
        "the restored batch is reported by the next flush"
    );
}
