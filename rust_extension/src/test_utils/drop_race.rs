//! A controlled publication race shared by the socket and HTTP drop tests.
//!
//! Both handlers report drops through the same [`RateLimitedWarner`], so the
//! mechanism that decides which of two reporters wins an expired interval is
//! worth pinning once. Only the cause labels, the rendered prefix and the
//! handler-specific assertions differ, so those stay in each handler's own
//! test.
//!
//! The race parks one caller inside its report-time sample, after it has
//! claimed the interval but before it has published a report time. That is the
//! window in which a real caller would be delayed, and choosing it explicitly
//! is what makes the winner a decision of the test rather than of the
//! scheduler.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use crate::rate_limited_warner::Clock;

/// A clock that parks one chosen sample until the test releases it.
pub(crate) struct ParkingClock {
    /// Current test-controlled timestamp in milliseconds.
    now_ms: AtomicU64,
    /// Samples taken so far across all callers.
    samples: AtomicU64,
    /// Index of the sample that parks, or `u64::MAX` while disarmed.
    park_at: AtomicU64,
    /// Signals that the parked caller has reached its sample.
    parked: mpsc::Sender<()>,
    /// Released by the test once the competing caller has reported.
    resume: Mutex<mpsc::Receiver<()>>,
}

impl Clock for ParkingClock {
    fn now_millis(&self) -> u64 {
        let sample = self.samples.fetch_add(1, Ordering::Relaxed);
        if sample == self.park_at.load(Ordering::Relaxed) {
            self.parked.send(()).expect("parked caller receiver");
            self.resume
                .lock()
                .expect("resume lock")
                .recv_timeout(Duration::from_secs(5))
                .expect("the test releases the parked caller");
        }
        self.now_ms.load(Ordering::Relaxed)
    }
}

impl ParkingClock {
    /// Move the clock to `now_ms`, which decides whether a report is due.
    pub(crate) fn set_now_millis(&self, now_ms: u64) {
        self.now_ms.store(now_ms, Ordering::Relaxed);
    }
}

/// One run of the controlled race, owning the channels behind it.
///
/// The clock is handed to the warner under test; the channels stay here so the
/// test only has to describe the two callers.
pub(crate) struct SupersededRace {
    /// The clock both callers sample.
    clock: Arc<ParkingClock>,
    /// Signalled when the parked caller reaches its sample.
    parked: mpsc::Receiver<()>,
    /// Releases the parked caller once the competitor has reported.
    resume: mpsc::Sender<()>,
}

impl Default for SupersededRace {
    fn default() -> Self {
        Self::new()
    }
}

impl SupersededRace {
    /// Build a race whose clock starts at zero and parks nothing yet.
    pub(crate) fn new() -> Self {
        let (parked_tx, parked_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        Self {
            clock: Arc::new(ParkingClock {
                now_ms: AtomicU64::new(0),
                samples: AtomicU64::new(0),
                park_at: AtomicU64::new(u64::MAX),
                parked: parked_tx,
                resume: Mutex::new(resume_rx),
            }),
            parked: parked_rx,
            resume: resume_tx,
        }
    }

    /// The shared clock, for moving time and for building the warner.
    pub(crate) fn clock(&self) -> Arc<ParkingClock> {
        Arc::clone(&self.clock)
    }

    /// Park `loser` after its claim, run `compete`, then release `loser`.
    ///
    /// The parked caller samples the clock once for its due pre-check and once
    /// for its claim, so its report-time sample is two indices after the run
    /// starts. `compete` runs on the test thread while the loser is parked; it
    /// must move the clock a whole interval on before reporting, so its report
    /// is due and supersedes the parked claim.
    pub(crate) fn run(self, loser: impl FnOnce() + Send, compete: impl FnOnce()) {
        let Self {
            clock,
            parked,
            resume,
        } = self;
        clock
            .park_at
            .store(clock.samples.load(Ordering::Relaxed) + 2, Ordering::Relaxed);
        thread::scope(|scope| {
            scope.spawn(loser);
            if parked.recv_timeout(Duration::from_secs(5)).is_err() {
                let _ = resume.send(());
                panic!("the first caller must park after claiming the interval");
            }
            compete();
            resume
                .send(())
                .expect("the parked caller receives its release");
        });
    }
}
