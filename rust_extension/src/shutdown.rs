//! A worker-completion signal that every closing thread can wait on.
//!
//! A handler's background worker is joined by whichever caller takes its join
//! handle first, but more than one thread may close the same handler at once.
//! A single completion message on a channel wakes one receiver only: the
//! second closer finds no join handle and no message and would return while the
//! worker was still writing queued records. An application that then tears down
//! the destination loses them.
//!
//! [`ShutdownCompletion`] broadcasts the one completion to every registered
//! waiter, and remembers it, so a closer that arrives after the worker has
//! stopped returns immediately instead of waiting out its timeout.

use std::{sync::Arc, time::Duration};

use crate::sync::{Mutex, Receiver, Sender, bounded, recv_timeout};

/// Shared completion state for one worker thread.
///
/// Created before the worker is spawned, signalled by the worker as it exits,
/// and consulted by every closer.
pub(crate) struct ShutdownCompletion {
    /// Completion flag and the waiters still to be woken.
    state: Mutex<CompletionState>,
}

/// Completion flag plus the waiters that have not yet been woken.
#[derive(Default)]
struct CompletionState {
    /// Whether the worker has already signalled completion.
    finished: bool,
    /// Channels handed to closers that registered before completion.
    waiters: Vec<Sender<()>>,
}

impl ShutdownCompletion {
    /// Create a completion that no worker has signalled yet.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let completion = ShutdownCompletion::new();
    /// assert!(!completion.wait(Duration::from_millis(1)));
    /// ```
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(CompletionState::default()),
        })
    }

    /// Register a receiver that is notified once the worker has stopped.
    ///
    /// A registration made after completion receives its notification at once,
    /// which is what lets a late closer return without waiting out the timeout.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let waiter = completion.register();
    /// assert_eq!(waiter.recv_timeout(Duration::from_secs(1)), Err(RecvTimeoutError::Timeout));
    /// ```
    pub(crate) fn register(&self) -> Receiver<()> {
        let (tx, rx) = bounded(1);
        let mut state = self.state.lock();
        if state.finished {
            drop(state);
            let _ = tx.send(());
        } else {
            state.waiters.push(tx);
        }
        rx
    }

    /// Record that the worker has stopped and wake every waiting closer.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// completion.complete();
    /// assert!(completion.wait(Duration::from_millis(1)));
    /// ```
    pub(crate) fn complete(&self) {
        let mut state = self.state.lock();
        state.finished = true;
        for waiter in state.waiters.drain(..) {
            let _ = waiter.send(());
        }
    }

    /// Wait for the worker to stop, or for the timeout to expire.
    ///
    /// Returns whether completion was observed.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// assert!(!completion.wait(Duration::from_millis(1)));
    /// ```
    pub(crate) fn wait(&self, timeout: Duration) -> bool {
        recv_timeout(&self.register(), timeout).is_ok()
    }
}

/// Signals a [`ShutdownCompletion`] when dropped, including while unwinding.
///
/// A worker that panics never reaches its closing statements. Without a guard,
/// every closer would wait out its whole timeout and report a worker that "did
/// not shut down" while the join handle still pointed at a dead thread.
pub(crate) struct CompleteOnDrop(Arc<ShutdownCompletion>);

impl CompleteOnDrop {
    /// Arms a guard that completes `completion` however the worker leaves.
    pub(crate) fn new(completion: Arc<ShutdownCompletion>) -> Self {
        Self(completion)
    }
}

impl Drop for CompleteOnDrop {
    fn drop(&mut self) {
        self.0.complete();
    }
}

#[cfg(test)]
mod tests {
    //! Completion-broadcast tests.

    use std::{panic::AssertUnwindSafe, sync::Arc, time::Duration};

    use super::{CompleteOnDrop, ShutdownCompletion};

    /// Every waiter registered before completion is woken.
    #[test]
    fn every_registered_waiter_observes_completion() {
        let completion = ShutdownCompletion::new();
        let first = completion.register();
        let second = completion.register();

        completion.complete();

        for waiter in [first, second] {
            waiter
                .recv_timeout(Duration::from_secs(1))
                .expect("a registered waiter must be woken by completion");
        }
    }

    /// A waiter that arrives after completion does not wait for the timeout.
    #[test]
    fn a_late_waiter_returns_immediately() {
        let completion = ShutdownCompletion::new();
        completion.complete();

        assert!(
            completion.wait(Duration::from_secs(30)),
            "a completed worker must satisfy a late waiter at once",
        );
    }

    /// An uncompleted worker leaves a waiter to time out.
    #[test]
    fn an_uncompleted_worker_times_out() {
        let completion = ShutdownCompletion::new();

        assert!(
            !completion.wait(Duration::from_millis(1)),
            "an unfinished worker must not report completion",
        );
    }

    /// Completion is idempotent and keeps waking later waiters.
    #[test]
    fn completion_can_be_signalled_twice() {
        let completion = ShutdownCompletion::new();
        completion.complete();
        completion.complete();

        assert!(
            completion.wait(Duration::from_secs(1)),
            "a repeated completion must still satisfy a later waiter",
        );
    }

    /// The guard completes when its scope ends normally.
    #[test]
    fn the_drop_guard_completes_on_a_normal_exit() {
        let completion = ShutdownCompletion::new();
        {
            let _signal = CompleteOnDrop::new(Arc::clone(&completion));
        }

        assert!(
            completion.wait(Duration::from_millis(1)),
            "leaving the guarded scope must signal completion",
        );
    }

    /// The guard completes while a panicking worker unwinds.
    #[test]
    fn the_drop_guard_completes_while_unwinding() {
        let completion = ShutdownCompletion::new();
        let worker_completion = Arc::clone(&completion);
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(move || {
            let _signal = CompleteOnDrop::new(worker_completion);
            panic!("simulated worker failure");
        }));

        assert!(outcome.is_err(), "the simulated worker must have panicked");
        assert!(
            completion.wait(Duration::from_millis(1)),
            "a panicking worker must still signal completion to its closers",
        );
    }
}
