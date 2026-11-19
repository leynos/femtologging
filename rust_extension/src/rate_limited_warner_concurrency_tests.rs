//! Concurrency contracts for `RateLimitedWarner`'s interval claim.
//!
//! One warner is shared by producers recording drops, a flush draining the
//! count, and the thread that wins the next warning interval. These tests hold
//! the interleavings that sharing must survive: a flush that drains a due call
//! does not claim its interval, a flush cannot take a batch a due call has
//! already drained, a clock that re-enters the warner does not deadlock its own
//! claim, and racers recording and draining conserve every drop - none lost,
//! none counted twice.
//!
//! The fixtures synchronize with barriers and channels rather than sleeping, so
//! a failure names the interleaving under test rather than the host's load.

use super::tests::FakeClock;
use super::{Clock, RateLimitedWarner};
use rstest::rstest;
use std::sync::Arc;
use std::sync::Barrier;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// A clock that pauses one chosen sample so the test can interleave with it.
struct PausingClock {
    /// Timestamp every sample returns.
    now_ms: u64,
    /// Samples taken so far, so the pause lands on the chosen one.
    samples: AtomicU64,
    /// Zero-based index of the sample that pauses.
    pause_on_sample: u64,
    /// Synchronizes the pausing caller with the test.
    entered: Barrier,
    /// Prevents the pausing caller from continuing until released.
    resume: Barrier,
}

impl PausingClock {
    /// Build a clock that pauses its `pause_on_sample`-th sample.
    fn new(now_ms: u64, pause_on_sample: u64) -> Self {
        Self {
            now_ms,
            samples: AtomicU64::new(0),
            pause_on_sample,
            entered: Barrier::new(2),
            resume: Barrier::new(2),
        }
    }
}

impl Clock for PausingClock {
    fn now_millis(&self) -> u64 {
        if self.samples.fetch_add(1, Ordering::Relaxed) == self.pause_on_sample {
            self.entered.wait();
            self.resume.wait();
        }
        self.now_ms
    }
}

/// A flush draining a due call does not leave an empty interval claimed.
///
/// The warning caller is paused inside its due pre-check, before it drains
/// anything. The flush takes and reports the only pending drop, so the resumed
/// caller finds nothing to report and must claim nothing: a claimed interval
/// with no report behind it would suppress the next drop for a whole interval.
#[test]
fn flush_draining_a_due_call_does_not_claim_its_interval() {
    let clock = Arc::new(PausingClock::new(1000, 0));
    let warner = Arc::new(RateLimitedWarner::with_clock(
        Duration::from_secs(1),
        Arc::clone(&clock) as Arc<dyn Clock>,
    ));
    warner.record_drop();

    let warning_counts = Arc::new(Mutex::new(Vec::new()));
    thread::scope(|scope| {
        let warning_warner = Arc::clone(&warner);
        let warning_counts = Arc::clone(&warning_counts);
        scope.spawn(move || {
            warning_warner.warn_if_due(|count| {
                warning_counts
                    .lock()
                    .expect("warning counts lock")
                    .push(count);
            });
        });
        clock.entered.wait();

        let mut flush_counts = Vec::new();
        warner.flush(|count| flush_counts.push(count));
        // Release the paused warning thread before asserting: a failed
        // assertion would otherwise unwind past the barrier and leave
        // `thread::scope` joining a thread that can never resume, turning a
        // regression into a hung test run instead of a failure.
        clock.resume.wait();
        assert_eq!(flush_counts, vec![1]);
        assert_eq!(warner.pending_drops(), 0);
    });

    assert!(
        warning_counts
            .lock()
            .expect("warning counts lock")
            .is_empty()
    );
    warner.record_drop();
    warner.warn_if_due(|count| {
        warning_counts
            .lock()
            .expect("warning counts lock")
            .push(count);
    });
    assert_eq!(
        *warning_counts.lock().expect("warning counts lock"),
        vec![1],
        "the flush must not leave the next drop suppressed"
    );
}

/// A flush cannot take a batch a due call has already drained.
///
/// The due caller has swapped the batch out and is paused before its claim, so
/// no interval is claimed yet when the flush runs. The flush must find nothing,
/// and the resumed caller must still claim the interval and report the batch it
/// holds: draining first is what keeps a flush from claiming an interval that
/// no report covers.
#[test]
fn flush_cannot_take_a_batch_a_due_call_already_drained() {
    let clock = Arc::new(PausingClock::new(1000, 1));
    let warner = Arc::new(RateLimitedWarner::with_clock(
        Duration::from_secs(1),
        Arc::clone(&clock) as Arc<dyn Clock>,
    ));
    warner.record_drop();

    let warning_counts = Arc::new(Mutex::new(Vec::new()));
    thread::scope(|scope| {
        let warning_warner = Arc::clone(&warner);
        let warning_counts = Arc::clone(&warning_counts);
        scope.spawn(move || {
            warning_warner.warn_if_due(|count| {
                warning_counts
                    .lock()
                    .expect("warning counts lock")
                    .push(count);
            });
        });
        clock.entered.wait();

        let mut flush_counts = Vec::new();
        warner.flush(|count| flush_counts.push(count));
        clock.resume.wait();
        assert!(
            flush_counts.is_empty(),
            "the batch belongs to the due caller that drained it"
        );
    });

    assert_eq!(
        *warning_counts.lock().expect("warning counts lock"),
        vec![1]
    );
    assert_eq!(warner.pending_drops(), 0);
    assert_eq!(
        warner.last_warn.load(Ordering::Relaxed),
        1000,
        "the caller that reported the batch is the one that claimed the interval"
    );
}

/// A clock that re-enters the warner while the due claim samples it.
///
/// The clock is a caller-supplied seam, so nothing stops an implementation from
/// touching the same warner: a shutdown hook that reads the time and then
/// flushes is enough. This double does exactly that on its second sample, which
/// is the one [`RateLimitedWarner::warn_if_due`] takes for its interval claim.
struct ReentrantClock {
    /// Timestamp every sample returns.
    now_ms: u64,
    /// Samples taken so far, so the re-entry lands on the claim's sample.
    samples: AtomicU64,
    /// Warner the clock re-enters, set once the warner exists.
    warner: OnceLock<Arc<RateLimitedWarner>>,
    /// Drops the re-entrant flush reported.
    flushed: AtomicU64,
}

impl Clock for ReentrantClock {
    fn now_millis(&self) -> u64 {
        // Sample zero is the lock-free pre-check. Sample one is the claim's,
        // taken after the caller has already drained its batch.
        if self.samples.fetch_add(1, Ordering::Relaxed) == 1
            && let Some(warner) = self.warner.get()
        {
            warner.flush(|count| {
                self.flushed.fetch_add(count, Ordering::Relaxed);
            });
        }
        self.now_ms
    }
}

/// A clock that re-enters the warner must not deadlock or lose the batch.
///
/// The warner holds nothing while sampling the clock, so the re-entrant flush
/// runs to completion. It finds the caller's batch already drained and reports
/// nothing; the caller then claims its interval and reports that batch itself.
/// The timeout below fails the test rather than hanging the suite if the
/// re-entry ever waits for state its own sample is needed to release.
#[test]
fn reentrant_clock_does_not_deadlock_the_interval_claim() {
    let clock = Arc::new(ReentrantClock {
        now_ms: 1_000,
        samples: AtomicU64::new(0),
        warner: OnceLock::new(),
        flushed: AtomicU64::new(0),
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

    let (done_tx, done_rx) = mpsc::channel();
    let warned = Arc::new(AtomicBool::new(false));
    let caller = {
        let warner = Arc::clone(&warner);
        let warned = Arc::clone(&warned);
        thread::spawn(move || {
            warner.warn_if_due(|_| warned.store(true, Ordering::Relaxed));
            let _ = done_tx.send(());
        })
    };
    done_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the due claim must not wait for state its own clock re-entry needs");
    caller.join().expect("the warning caller should finish");

    assert_eq!(
        clock.flushed.load(Ordering::Relaxed),
        0,
        "the re-entrant flush must find the batch already drained"
    );
    assert!(
        warned.load(Ordering::Relaxed),
        "the caller that drained the batch must report it"
    );
    assert_eq!(warner.pending_drops(), 0);
}

/// Shared state every conservation worker reads and updates.
struct ConservationRun<'a> {
    /// Warner under test.
    warner: &'a RateLimitedWarner,
    /// Clock the capturing thread advances.
    clock: &'a FakeClock,
    /// Drops the threads have recorded, counted by the threads themselves.
    recorded: &'a AtomicU64,
    /// Drops the callbacks have reported.
    reported: &'a AtomicU64,
    /// Set if any callback is handed an empty batch.
    saw_empty_batch: &'a AtomicBool,
    /// Releases every worker at the same instant.
    start: &'a Barrier,
}

impl ConservationRun<'_> {
    /// Exercise one thread's share of the mixed workload.
    fn work(&self, index: usize, ops: usize) {
        self.start.wait();
        let mut warn = |count: u64| self.report(count);
        for step in 0..ops {
            self.warner.record_drop();
            self.recorded.fetch_add(1, Ordering::Relaxed);
            match step % 3 {
                0 => self.warner.warn_if_due(&mut warn),
                1 if index == 0 => self.clock.advance(1),
                _ => self.warner.flush(&mut warn),
            }
        }
    }

    /// Account for one callback batch, flagging an empty one.
    fn report(&self, count: u64) {
        if count == 0 {
            self.saw_empty_batch.store(true, Ordering::Relaxed);
        }
        self.reported.fetch_add(count, Ordering::Relaxed);
    }
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
    let warner = RateLimitedWarner::with_clock(
        Duration::from_millis(1),
        Arc::clone(&clock) as Arc<dyn Clock>,
    );
    let recorded = AtomicU64::new(0);
    let reported = AtomicU64::new(0);
    let saw_empty_batch = AtomicBool::new(false);
    let start = Barrier::new(thread_count);
    let run = ConservationRun {
        warner: &warner,
        clock: clock.as_ref(),
        recorded: &recorded,
        reported: &reported,
        saw_empty_batch: &saw_empty_batch,
        start: &start,
    };

    thread::scope(|scope| {
        for index in 0..thread_count {
            let run = &run;
            scope.spawn(move || run.work(index, ops));
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
