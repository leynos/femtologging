//! A handler worker a test supplies instead of one the handler spawns.
//!
//! The close and flush contracts are decided by what a worker does, not by
//! what the handler asked of it, so a test builds the worker and the signal
//! that reaches it. Keeping the pair in one place leaves a handler's test
//! constructor with the two things a caller actually chooses: the queue it
//! writes into and the worker that drains it.

use std::sync::Arc;
use std::thread;

use crate::shutdown_signal::ShutdownSignal;

/// A worker thread together with the signal that asks it to stop.
///
/// The two travel together because neither is useful alone: a test that
/// supplies a thread without a signal leaves the handler unable to ask it to
/// stop, and a signal with no thread has nothing to observe.
pub(crate) struct TestWorker {
    /// The signal the handler raises to ask the worker to stop.
    pub(crate) signal: Arc<ShutdownSignal>,
    /// The worker thread, joined or abandoned by the close path.
    pub(crate) handle: thread::JoinHandle<()>,
}

impl TestWorker {
    /// Pair a worker thread with the signal it waits on.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let signal = Arc::new(ShutdownSignal::new());
    /// let handle = thread::spawn(|| {});
    /// let worker = TestWorker::new(signal, handle);
    /// assert!(Arc::strong_count(&worker.signal) >= 1);
    /// ```
    pub(crate) fn new(signal: Arc<ShutdownSignal>, handle: thread::JoinHandle<()>) -> Self {
        Self { signal, handle }
    }
}
