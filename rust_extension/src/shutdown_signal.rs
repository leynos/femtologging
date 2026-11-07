//! Shutdown state shared between a handler and the worker it owns.
//!
//! A worker waits on its command channel, and the records arriving there are
//! exactly what it must stop doing when the handler closes. A shutdown request
//! sent through that same channel therefore queues behind the backlog it is
//! meant to cancel: a worker on its thousandth retry does not read it until the
//! record in hand has exhausted its backoff. The waits inside that record's
//! network calls are just as deaf, and a stalled collector can hold each one
//! for the whole configured timeout.
//!
//! This module is the second channel those workers can hear. It carries no
//! work, only a phase, and [`ShutdownSignal::wait`] is the interruptible
//! stand-in for `thread::sleep` that a worker uses wherever it would otherwise
//! sleep through a close.
//!
//! # Phases
//!
//! - [`Phase::Running`] - ordinary operation. The handler has not asked for
//!   anything, and a clamped timeout is the configured one.
//! - [`Phase::Draining`] - a graceful shutdown with a deadline. Backoff sleeps
//!   end, each pending record gets one attempt, and every network timeout is
//!   clamped to what remains of the budget.
//! - [`Phase::Aborted`] - the deadline passed, or the handler gave up on the
//!   acknowledgement. Nothing further is attempted.
//!
//! # The deadline bounds what a worker starts
//!
//! A worker cannot be stopped mid-`connect`: the standard library has no
//! cancellation for a blocking socket call. What this module bounds is what the
//! worker begins. Nothing new starts once a drain deadline has passed or the
//! phase is aborted, and nothing started after a drain began runs longer than
//! the budget's remainder. Termination is therefore bounded by the budget plus
//! one network call already in flight when the deadline fell - the figure the
//! handler's `close` already allows for when it declines to join.

use std::time::{Duration, Instant};

#[cfg(test)]
use std::sync::mpsc;

use crate::monotonic_clock::{SharedClock, system_clock};
use crate::sync::{Condvar, Mutex};

/// What the handler has asked its worker to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Ordinary operation.
    Running,
    /// Drain with a deadline, then stop.
    Draining {
        /// The point after which no further work is started.
        deadline: Instant,
    },
    /// Stop immediately, abandoning anything not already sent.
    Aborted,
}

/// Why [`ShutdownSignal::wait`] returned before its delay elapsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    /// The delay elapsed with no change in phase. The wait was a plain sleep.
    Elapsed,
    /// A graceful shutdown began, or was already under way. The caller owns the
    /// decision to retry; the signal is telling it that a decision is due.
    ShutdownRequested,
    /// The signal is aborted, or the drain deadline has passed. The caller must
    /// stop rather than sleep out the rest of its delay.
    Aborted,
}

/// Phase plus the notification the workers park on.
///
/// Shared as an `Arc` between a handler and its worker: the handler moves the
/// phase on, and the worker waits for it between attempts.
pub struct ShutdownSignal {
    /// Current lifecycle phase used to bound and cancel worker operations.
    phase: Mutex<Phase>,
    /// Notified on every phase change. The phase is mutated under the guard and
    /// the notification is sent while still holding it, which is the
    /// discipline the standard library's condition-variable examples use: it
    /// leaves no window in which a waiter has released the lock but has not yet
    /// parked, and so cannot miss the change it is waiting to observe.
    changed: Condvar,
    /// Where the deadline queries read "now" from.
    ///
    /// Sampled once per query rather than captured at construction: the phase
    /// changes while the signal lives, and each query's answer has to be
    /// against the moment it was asked. Tests supply a clock that only moves
    /// when told to, so a deadline boundary is a value they choose rather than
    /// a race against the host's scheduler.
    ///
    /// One instant per query, and never two: a query that sampled the clock,
    /// decided, and sampled again could return a value belonging to neither
    /// moment. That is why the deadline predicates here read the clock exactly
    /// once, and why a test asserts against
    /// [`phase_and_now`](ShutdownSignal::phase_and_now) rather than calling
    /// the queries piecemeal.
    clock: SharedClock,
    /// Fires once per [`wait`](ShutdownSignal::wait), at the moment the wait
    /// begins.
    ///
    /// `wait` blocks for as long as its caller asked, and no amount of polling
    /// from outside can tell a wait that is about to be interrupted from one
    /// already sleeping. A test asserting that a close is observed *during* a
    /// backoff has to know the worker reached the backoff first, and a sleep
    /// long enough to guess that on a loaded host is a test that measures the
    /// host. This channel is that knowledge.
    ///
    /// Notification only: the worker is never held here. An earlier version
    /// also parked the worker until the test released it, which put that
    /// hand-off inside the handler's shutdown budget and made a correct close
    /// look like one that had missed its deadline.
    #[cfg(test)]
    waiting: Mutex<Option<mpsc::Sender<()>>>,
}

impl ShutdownSignal {
    /// Create a signal in the [`Phase::Running`] phase.
    pub fn new() -> Self {
        Self {
            phase: Mutex::new(Phase::Running),
            changed: Condvar::new(),
            clock: system_clock(),
            #[cfg(test)]
            waiting: Mutex::new(None),
        }
    }

    /// Create a signal whose deadlines are read from `clock`.
    ///
    /// The production constructor already uses the real clock; this one exists
    /// so a test can make a deadline boundary depend on the instant it chose
    /// rather than on how quickly the test thread got scheduled. A deadline
    /// built from the clock's own base is the natural pairing, and
    /// [`ManualClock::base`](crate::monotonic_clock::ManualClock::base) is
    /// there to express it.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let clock = Arc::new(ManualClock::new());
    /// let signal = ShutdownSignal::with_clock(Arc::clone(&clock));
    /// ```
    #[cfg(test)]
    pub(crate) fn with_clock(clock: SharedClock) -> Self {
        Self {
            phase: Mutex::new(Phase::Running),
            changed: Condvar::new(),
            clock,
            #[cfg(test)]
            waiting: Mutex::new(None),
        }
    }

    /// Begin a graceful shutdown that must be complete by `deadline`.
    ///
    /// Wakes every waiting worker. A second call, or a call after
    /// [`abort`](ShutdownSignal::abort), does nothing: the deadline that binds
    /// a shutdown is the one the handler chose.
    pub fn begin_shutdown(&self, deadline: Instant) {
        let mut phase = self.phase.lock();
        if *phase != Phase::Running {
            return;
        }
        *phase = Phase::Draining { deadline };
        self.changed.notify_all();
    }

    /// Abandon anything not already sent.
    ///
    /// Called by the handler when the acknowledgement deadline passes without
    /// an answer, which is this shutdown's own deadline arriving by the only
    /// route left: a worker too busy to acknowledge is a worker too busy to
    /// honour a longer drain. No clock is consulted here, because the handler
    /// has already decided.
    pub fn abort(&self) {
        let mut phase = self.phase.lock();
        if *phase == Phase::Aborted {
            return;
        }
        *phase = Phase::Aborted;
        self.changed.notify_all();
    }

    /// Whether a graceful shutdown has begun.
    pub fn is_shutting_down(&self) -> bool {
        !matches!(self.current(), Phase::Running)
    }

    /// Whether the worker should stop rather than start anything further.
    ///
    /// True in the [`Phase::Aborted`] phase, and true once a drain deadline has
    /// passed. A worker asks before each item of work it is about to begin, so
    /// that the deadline bounds the work started after it, not only the work
    /// already running.
    pub fn is_aborted(&self) -> bool {
        match self.current() {
            Phase::Running => false,
            Phase::Draining { deadline } => deadline <= self.clock.now(),
            Phase::Aborted => true,
        }
    }

    /// What remains of the drain budget.
    ///
    /// `None` while the signal is running, because no budget applies, and
    /// `None` once the budget is spent or the signal is aborted. Callers that
    /// need to tell those apart ask [`is_shutting_down`] first; callers that
    /// only want to know how long an operation may take want
    /// [`clamp`](ShutdownSignal::clamp), which already knows the difference.
    ///
    /// A remainder of exactly zero is reported as `None`, not as `Some(0)`.
    /// The instant the deadline arrives is the boundary this query exists to
    /// police, and `checked_duration_since` answers `Some(0)` there rather than
    /// `None`: the deadline is not in the past, it has just been reached. Those
    /// two must not disagree with [`is_aborted`](ShutdownSignal::is_aborted),
    /// which treats the arriving instant as aborted, and a caller handed a zero
    /// budget must not start: `std` and `ureq` both read a zero timeout as "no
    /// timeout", the unbounded wait a spent budget forbids.
    ///
    /// [`is_aborted`]: ShutdownSignal::is_aborted
    pub fn remaining(&self) -> Option<Duration> {
        match self.current() {
            Phase::Running | Phase::Aborted => None,
            Phase::Draining { deadline } => deadline
                .checked_duration_since(self.clock.now())
                .filter(|left| !left.is_zero()),
        }
    }

    /// Bound `configured` by what is left of the shutdown budget.
    ///
    /// `None` means the budget is spent and the caller must not begin the
    /// operation at all, rather than begin it with a timeout of zero: `std` and
    /// `ureq` both read a zero duration as "no timeout" rather than "no time",
    /// which is the opposite of what a spent budget is asking for.
    pub fn clamp(&self, configured: Duration) -> Option<Duration> {
        match self.current() {
            Phase::Running => Some(configured),
            Phase::Draining { .. } => self.remaining().map(|left| left.min(configured)),
            Phase::Aborted => None,
        }
    }

    /// Sleep for `delay`, waking early if the phase changes.
    ///
    /// The replacement for every `thread::sleep` on a worker's retry path. A
    /// retry delay postpones the next attempt while running. If graceful
    /// shutdown starts during that delay, the caller should stop retrying the
    /// current record and let the worker apply its drain policy. If shutdown
    /// aborts or the drain deadline expires, the caller must stop work.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let signal = std::sync::Arc::new(ShutdownSignal::new());
    /// let waiting = std::thread::spawn({
    ///     let signal = std::sync::Arc::clone(&signal);
    ///     move || signal.wait(Duration::from_secs(30))
    /// });
    /// // A shutdown interrupts a sleep that had thirty seconds to run.
    /// signal.begin_shutdown(std::time::Instant::now() + Duration::from_secs(5));
    /// assert_eq!(waiting.join().unwrap(), WaitOutcome::ShutdownRequested);
    /// ```
    pub fn wait(&self, delay: Duration) -> WaitOutcome {
        #[cfg(test)]
        self.announce_wait();
        // Two clocks on purpose. Whether the drain deadline has passed is the
        // same question `is_aborted` answers, so it reads the signal's clock
        // and the two can never disagree. The sleep bound is a real duration
        // handed to `wait_timeout`, which has no seam: a manual clock that
        // never advances would leave `left` constant and this loop unable to
        // finish.
        let ends_at = Instant::now() + delay;
        let mut phase = self.phase.lock();
        loop {
            match *phase {
                Phase::Running => {}
                Phase::Draining { deadline } => {
                    return if self.clock.now() >= deadline {
                        WaitOutcome::Aborted
                    } else {
                        WaitOutcome::ShutdownRequested
                    };
                }
                Phase::Aborted => return WaitOutcome::Aborted,
            }
            // A wakeup that is not a phase change leaves the same phase to be
            // matched again; this check is what makes that loop terminate.
            let Some(left) = ends_at.checked_duration_since(Instant::now()) else {
                return WaitOutcome::Elapsed;
            };
            // The guard is consumed by the wait and the lock is held again
            // when it returns, so the phase is re-read rather than assumed.
            // Both the ordinary and the Loom arm of the seam report whether
            // the wait timed out; neither answer is read here, because the
            // phase is the only thing that decides what happens next.
            self.changed.wait_timeout(phase, left);
            phase = self.phase.lock();
        }
    }

    /// Ask to be told when a worker next begins waiting.
    ///
    /// Returns a receiver that yields once, when the next
    /// [`wait`](ShutdownSignal::wait) call begins. The registration is spent
    /// when it fires, so a test that observes one wait is told about one wait
    /// and cannot be satisfied by a later retry's sleep.
    ///
    /// The worker is not held at the wait: it goes on to observe the phase as
    /// usual. A test that wants the close to land *while* the worker is in the
    /// delay uses a delay far longer than the shutdown budget, so the worker is
    /// still waiting when the shutdown arrives however quickly the test thread
    /// runs. Holding it instead would fold the test's own hand-off into the
    /// budget under test.
    ///
    /// This exists only for tests, and is compiled only for them.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let signal = ShutdownSignal::new();
    /// let waits = signal.notify_on_wait();
    /// let worker = std::thread::spawn({ /* ... */ signal.wait(one_second) });
    /// waits.recv_timeout(one_second).expect("the worker should be sleeping");
    /// ```
    #[cfg(test)]
    pub fn notify_on_wait(&self) -> mpsc::Receiver<()> {
        let (tx, rx) = mpsc::channel();
        *self.waiting.lock() = Some(tx);
        rx
    }

    /// Tell the test that registered with [`notify_on_wait`] that a wait has
    /// begun.
    ///
    /// [`notify_on_wait`]: ShutdownSignal::notify_on_wait
    #[cfg(test)]
    fn announce_wait(&self) {
        if let Some(started) = self.waiting.lock().take() {
            let _ = started.send(());
        }
    }

    /// Return a snapshot of the phase while holding the state lock briefly.
    ///
    /// Example: `is_aborted` treats an expired drain snapshot as aborted.
    fn current(&self) -> Phase {
        *self.phase.lock()
    }

    /// The phase and the instant the signal's clock reads now, read together.
    ///
    /// Test-only. The deadline queries each sample the clock, so a test that
    /// called them separately and compared the answers would be comparing two
    /// different instants - which is exactly the race the manual clock exists
    /// to remove. This returns the phase with the instant it should be judged
    /// against, so an invariant can be stated once and checked exactly.
    #[cfg(test)]
    pub(crate) fn phase_and_now(&self) -> (Phase, Instant) {
        (self.current(), self.clock.now())
    }
}

impl Default for ShutdownSignal {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ShutdownSignal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShutdownSignal")
            .field("phase", &self.current())
            .finish()
    }
}

#[cfg(test)]
#[path = "shutdown_signal_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "shutdown_signal_property_tests.rs"]
mod property_tests;
