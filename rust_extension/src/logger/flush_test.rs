//! Test-only synchronization hooks for Python handler flush regression tests.
//!
//! The flush tests need to observe the point where the whole barrier waits on
//! the worker and to keep a handler blocked until they release it. The logger
//! signals that point: `FemtoLogger::notify_barrier_waiting` runs once the shard
//! has admitted the marker and started the worker, immediately before the
//! acknowledgement wait. `FlushTestHooks` carries the handshake, the test waits
//! for the signal and can cancel a wait that nothing will satisfy. The
//! `wait_for_flush_entry_for_test`, `drain_flush_signals_for_test` and
//! `cancel_flush_wait_for_test` helpers wire the hooks to a live `FemtoLogger`
//! for the Python tests, which reach them through the extension module, and the
//! `tests` submodule covers the signal bookkeeping itself.

use std::time::Duration;

use pyo3::prelude::*;

use crate::sync::{Receiver, Sender, bounded};

use super::FemtoLogger;

/// Message sent when a token-free flush method starts or a test wait is cancelled.
enum FlushTestSignal {
    /// The token-free flush method has started.
    Entered,
    /// Cancel a pending test wait during cleanup.
    Cancel,
}

/// Per-logger signal channel used only by test-util builds.
pub(crate) struct FlushTestHooks {
    /// Sends flush-entry and cancellation notifications.
    signal_tx: Sender<FlushTestSignal>,
    /// Receives flush-entry and cancellation notifications.
    signal_rx: Receiver<FlushTestSignal>,
}

impl FlushTestHooks {
    /// Create a one-slot channel for bounded test coordination.
    pub(crate) fn new() -> Self {
        let (signal_tx, signal_rx) = bounded(1);
        Self {
            signal_tx,
            signal_rx,
        }
    }

    /// Notify the test waiter that the token-free flush method has started.
    pub(crate) fn signal_entered(&self) {
        // A full slot already holds a signal a waiter will read, and a
        // disconnected one means no test is waiting; neither needs a retry,
        // so the refusal is recorded rather than repaired.
        if self.signal_tx.try_send(FlushTestSignal::Entered).is_err() {
            log::trace!("FlushTestHooks: entry signal had no room or no waiter");
        }
    }

    /// Discard signals from earlier flushes before a test arms its gate.
    pub(crate) fn drain_pending_signals(&self) {
        while self.signal_rx.try_recv().is_ok() {}
    }

    /// Wait for flush entry or cancellation for at most `timeout_ms`.
    pub(crate) fn wait_for_entry(&self, timeout_ms: u64) -> bool {
        matches!(
            self.signal_rx
                .recv_timeout(Duration::from_millis(timeout_ms)),
            Ok(FlushTestSignal::Entered)
        )
    }

    /// Wake a pending test wait during cleanup.
    pub(crate) fn cancel_wait(&self) {
        // A full slot already holds a signal that will wake the waiter, and a
        // disconnected one means nobody is waiting; either way the cancel has
        // nothing to add, so the refusal is recorded.
        if self.signal_tx.try_send(FlushTestSignal::Cancel).is_err() {
            log::trace!("FlushTestHooks: cancel signal had no room or no waiter");
        }
    }
}

/// Wait for a logger's flush-entry signal while releasing the GIL.
#[pyfunction]
pub(crate) fn wait_for_flush_entry_for_test(
    py: Python<'_>,
    logger: &Bound<'_, FemtoLogger>,
    timeout_ms: u64,
) -> bool {
    let borrowed = logger.borrow();
    let logger_ref: &FemtoLogger = &borrowed;
    py.detach(|| logger_ref.flush_test_hooks.wait_for_entry(timeout_ms))
}

/// Clear prior flush signals before a test arms a new handler gate.
#[pyfunction]
pub(crate) fn drain_flush_signals_for_test(logger: &Bound<'_, FemtoLogger>) {
    logger.borrow().flush_test_hooks.drain_pending_signals();
}

/// Cancel a logger's pending flush-entry test wait during cleanup.
#[pyfunction]
pub(crate) fn cancel_flush_wait_for_test(logger: &Bound<'_, FemtoLogger>) {
    logger.borrow().flush_test_hooks.cancel_wait();
}

#[cfg(test)]
mod tests {
    //! Regression checks for stale test-only flush-entry signals.

    use super::FlushTestHooks;

    #[test]
    fn draining_pending_signals_discards_a_stale_signal() {
        let hooks = FlushTestHooks::new();
        hooks.signal_entered();
        hooks.drain_pending_signals();

        assert!(!hooks.wait_for_entry(1));
    }
}
