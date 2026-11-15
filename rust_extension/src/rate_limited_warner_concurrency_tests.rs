//! Tests for atomic interval claims and synchronized warning drains.

use super::{Clock, ForkSafeDrainLock, RateLimitedWarner};
use std::sync::Arc;
use std::sync::Barrier;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// A clock whose timestamp never moves during a test.
struct FixedClock(u64);

impl Clock for FixedClock {
    fn now_millis(&self) -> u64 {
        self.0
    }
}

/// A clock that pauses the first warning caller inside its time sample.
struct PausingClock {
    /// Timestamp returned after the test releases the warning caller.
    now_ms: u64,
    /// Pauses only the first clock sample.
    should_pause: AtomicBool,
    /// Synchronizes the warning caller with the test before it pauses.
    entered: Barrier,
    /// Prevents the warning caller from sampling time until released.
    resume: Barrier,
}

impl PausingClock {
    /// Create the two-party barriers used by the race test.
    fn new(now_ms: u64) -> Self {
        Self {
            now_ms,
            should_pause: AtomicBool::new(true),
            entered: Barrier::new(2),
            resume: Barrier::new(2),
        }
    }
}

impl Clock for PausingClock {
    fn now_millis(&self) -> u64 {
        if self.should_pause.swap(false, Ordering::Relaxed) {
            self.entered.wait();
            self.resume.wait();
        }
        self.now_ms
    }
}

/// A flush winning the race does not leave an empty interval claimed.
#[test]
fn flush_draining_a_due_call_does_not_claim_its_interval() {
    let clock = Arc::new(PausingClock::new(1000));
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

/// A suppressed call returns without contending for the drain lock.
///
/// Every refused record calls `warn_if_due` while the interval is still open.
/// That path must not serialize over the mutex that also guards flushes: here
/// the test holds the lock, and the suppressed call still completes and leaves
/// the pending count untouched.
#[test]
fn suppressed_call_does_not_wait_for_the_drain_lock() {
    let warner = Arc::new(RateLimitedWarner::with_clock(
        Duration::from_secs(60),
        Arc::new(FixedClock(1_000)) as Arc<dyn Clock>,
    ));
    warner.record_drop();
    warner.warn_if_due(|_| {});
    warner.record_drop();

    let warned = Arc::new(AtomicBool::new(false));
    let (done_tx, done_rx) = mpsc::channel();
    let lock = warner.drain_lock.lock();
    let caller = {
        let warner = Arc::clone(&warner);
        let warned = Arc::clone(&warned);
        thread::spawn(move || {
            warner.warn_if_due(|_| warned.store(true, Ordering::Relaxed));
            let _ = done_tx.send(());
        })
    };
    let completed = done_rx.recv_timeout(Duration::from_secs(10));
    drop(lock);
    completed.expect("a suppressed call must not wait for the drain lock");
    caller.join().expect("the suppressed caller should finish");

    assert!(
        !warned.load(Ordering::Relaxed),
        "the call arrived inside the interval and must not warn"
    );
    assert_eq!(
        warner.pending_drops(),
        1,
        "a suppressed call must leave the count pending"
    );
}

/// A live thread of the same process keeps the drain lock held.
#[test]
fn drain_lock_is_held_against_the_owning_process() {
    let lock = ForkSafeDrainLock::new();
    let guard = lock.try_lock(7).expect("a free lock should be takeable");
    assert!(
        lock.try_lock(7).is_none(),
        "a second caller in the same process must not take the lock",
    );
    drop(guard);
    assert!(
        lock.try_lock(7).is_some(),
        "dropping the guard must release the lock",
    );
}

/// A forked child reclaims the drain lock instead of deadlocking on it.
///
/// The parent can fork while another thread holds the lock. That thread does
/// not exist in the child, but a parking-lot mutex would stay locked forever;
/// the lock records its owner so a child can tell the difference.
#[test]
fn drain_lock_inherited_from_another_process_is_reclaimed() {
    let lock = ForkSafeDrainLock::new();
    let parent_guard = lock
        .try_lock(41)
        .expect("the parent should take the free lock");

    let child_guard = lock
        .try_lock(42)
        .expect("a child process should reclaim an inherited lock");
    // The parent's guard object survives into the child too. Dropping it there
    // must not release the lock the child now owns: its stored owner is the
    // parent's process ID, which no longer matches the word.
    drop(parent_guard);
    assert!(
        lock.try_lock(42).is_none(),
        "the stale guard must not free the child's lock",
    );

    drop(child_guard);
    assert!(
        lock.try_lock(42).is_some(),
        "dropping the child's guard must release the lock",
    );
}
