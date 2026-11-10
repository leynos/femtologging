//! Test-only accessors for [`FemtoSocketHandler`].
//!
//! Split out of `handler.rs` because `AGENTS.md` caps a code file at 400 lines
//! and the product code there had reached it. These are a child module of the
//! handler, which is what lets them read its private fields without widening
//! any of them: Rust privacy reaches a module *and its descendants*.
//!
//! Nothing here is compiled outside a test build, and nothing here is part of
//! the handler's contract with its worker. They exist because the close
//! contracts are about *when* a worker reaches a state, and a test that sleeps
//! long enough to guess the timing measures the host rather than the handler.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crossbeam_channel::Sender;
use parking_lot::Mutex;

use super::{FemtoSocketHandler, SocketCommand};
use crate::rate_limited_warner::RateLimitedWarner;
use crate::shutdown_signal::ShutdownSignal;

#[cfg(test)]
#[path = "flush_deadline_tests.rs"]
mod flush_deadline_tests;

impl FemtoSocketHandler {
    /// Build a handler around a worker the caller supplies.
    ///
    /// The flush and close contracts are about what happens when the worker
    /// cannot answer: the command queue stays full, or nothing reads it at
    /// all. Constructing that worker is the only deterministic way to test
    /// them, because a live worker races the test for the same window.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let (tx, _rx) = crossbeam_channel::bounded(1);
    /// let signal = Arc::new(ShutdownSignal::new());
    /// let handle = thread::spawn(|| {});
    /// let handler = FemtoSocketHandler::with_worker_for_test(
    ///     tx, signal, handle, Duration::from_millis(200),
    ///     RateLimitedWarner::new(Duration::from_secs(1)),
    /// );
    /// ```
    pub(crate) fn with_worker_for_test(
        tx: Sender<SocketCommand>,
        signal: Arc<ShutdownSignal>,
        handle: thread::JoinHandle<()>,
        flush_timeout: Duration,
        warner: RateLimitedWarner,
    ) -> Self {
        Self {
            tx: Some(tx),
            handle: Mutex::new(Some(handle)),
            warner,
            signal: Some(signal),
            flush_timeout,
            abandoned: Mutex::new(None),
        }
    }
    /// Ask to be told when the worker next begins a backoff delay.
    ///
    /// A test that asserts a close is observed *during* a retry delay has to
    /// know the worker reached that delay first; the alternative is a sleep
    /// long enough to guess it, which measures the host rather than the
    /// handler.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let backoffs = handler.wait_notification();
    /// handler.handle(record).expect("queue the record");
    /// backoffs.recv_timeout(one_second).expect("the worker should be backing off");
    /// ```
    pub(crate) fn wait_notification(&self) -> std::sync::mpsc::Receiver<()> {
        self.signal
            .as_ref()
            .expect("an open handler has a signal")
            .notify_on_wait()
    }

    /// Report when the worker this handler gave up on finally exits.
    ///
    /// Only a handler that took the abandonment path has a worker to report on.
    /// The returned receiver yields once the thread finishes; the thread is
    /// never joined by the handler, because that is the wait `close` declined
    /// to perform. Here a throwaway thread joins it purely to observe the exit.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let worker = handler.worker_for_test();
    /// handler.close();
    /// worker.recv_timeout(one_second).expect("the detached worker should stop");
    /// ```
    pub(crate) fn worker_for_test(&self) -> std::sync::mpsc::Receiver<()> {
        let (exited_tx, exited_rx) = std::sync::mpsc::channel();
        let handle = self.abandoned.lock().take();
        if let Some(handle) = handle {
            thread::spawn(move || {
                let _ = handle.join();
                let _ = exited_tx.send(());
            });
        }
        exited_rx
    }

    /// Clone the worker signal so a test can observe shutdown before releasing
    /// a worker paused at the backoff boundary.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let signal = handler.signal_for_test();
    /// assert!(!signal.is_shutting_down());
    /// ```
    pub(crate) fn signal_for_test(&self) -> Arc<ShutdownSignal> {
        Arc::clone(self.signal.as_ref().expect("an open handler has a signal"))
    }
}
