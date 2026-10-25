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
    /// Fires once per [`wait`](ShutdownSignal::wait), at the moment the wait
    /// begins.
    ///
    /// `wait` blocks for as long as its caller asked, and no amount of polling
    /// from outside can tell a wait that is about to be interrupted from one
    /// already sleeping. A test asserting that a close is observed *during* a
    /// backoff has to know the worker reached the backoff first, and a sleep
    /// long enough to guess that on a loaded host is a test that measures the
    /// host. This channel is that knowledge.
    #[cfg(test)]
    waiting: Mutex<Option<WaitHook>>,
}

/// Test coordination for observing and pausing a worker's next timed wait.
#[cfg(test)]
struct WaitHook {
    /// Notifies the test when the worker reaches the wait.
    started: mpsc::Sender<()>,
    /// Releases the worker when the test has observed shutdown.
    resume: Option<mpsc::Receiver<()>>,
}

impl ShutdownSignal {
    /// Create a signal in the [`Phase::Running`] phase.
    pub fn new() -> Self {
        Self {
            phase: Mutex::new(Phase::Running),
            changed: Condvar::new(),
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
            Phase::Draining { deadline } => deadline <= Instant::now(),
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
    /// [`is_shutting_down`]: ShutdownSignal::is_shutting_down
    pub fn remaining(&self) -> Option<Duration> {
        match self.current() {
            Phase::Running | Phase::Aborted => None,
            Phase::Draining { deadline } => deadline.checked_duration_since(Instant::now()),
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
    /// retry delay is a promise not to act yet, so a shutdown that interrupts
    /// one takes nothing that was owed: the caller learns it should retry
    /// rather than stop, in time to be told otherwise.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let signal = ShutdownSignal::new();
    /// let waiting = std::thread::spawn({
    ///     let signal = signal.clone();
    ///     move || signal.wait(Duration::from_secs(30))
    /// });
    /// // A shutdown interrupts a sleep that had thirty seconds to run.
    /// signal.begin_shutdown(std::time::Instant::now() + Duration::from_secs(5));
    /// assert_eq!(waiting.join().unwrap(), WaitOutcome::ShutdownRequested);
    /// ```
    pub fn wait(&self, delay: Duration) -> WaitOutcome {
        #[cfg(test)]
        self.announce_wait();
        let ends_at = Instant::now() + delay;
        let mut phase = self.phase.lock();
        loop {
            match *phase {
                Phase::Running => {}
                Phase::Draining { deadline } => {
                    return if Instant::now() >= deadline {
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
        *self.waiting.lock() = Some(WaitHook {
            started: tx,
            resume: None,
        });
        rx
    }

    /// Pause the next wait until the test releases it.
    ///
    /// This lets a test place a shutdown between a worker reaching backoff and
    /// the worker entering its timed wait, without relying on the backoff's
    /// random jitter to leave enough time for the test thread to run.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let (started, resume) = signal.pause_next_wait();
    /// // Wait until the worker reports that it reached backoff.
    /// started.recv().expect("worker reached the wait");
    /// resume.send(()).expect("resume the worker");
    /// ```
    #[cfg(test)]
    pub fn pause_next_wait(&self) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (started_tx, started_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        *self.waiting.lock() = Some(WaitHook {
            started: started_tx,
            resume: Some(resume_rx),
        });
        (started_rx, resume_tx)
    }

    /// Tell the test that registered with [`notify_on_wait`] that a wait has
    /// begun.
    ///
    /// [`notify_on_wait`]: ShutdownSignal::notify_on_wait
    #[cfg(test)]
    fn announce_wait(&self) {
        let hook = self.waiting.lock().take();
        if let Some(hook) = hook {
            let _ = hook.started.send(());
            if let Some(resume) = hook.resume {
                let _ = resume.recv();
            }
        }
    }

    /// Return a snapshot of the phase while holding the state lock briefly.
    ///
    /// Example: `is_aborted` treats an expired drain snapshot as aborted.
    fn current(&self) -> Phase {
        *self.phase.lock()
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
