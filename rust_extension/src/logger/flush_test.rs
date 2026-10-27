//! Test-only synchronization hooks for Python handler flush regression tests.

use std::time::Duration;

use pyo3::{PyRef, prelude::*};

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
        let _ = self.signal_tx.try_send(FlushTestSignal::Entered);
    }

    /// Wait for flush entry or cancellation for at most *timeout_ms*.
    pub(crate) fn wait_for_entry(&self, timeout_ms: u64) -> bool {
        matches!(
            self.signal_rx
                .recv_timeout(Duration::from_millis(timeout_ms)),
            Ok(FlushTestSignal::Entered)
        )
    }

    /// Wake a pending test wait during cleanup.
    pub(crate) fn cancel_wait(&self) {
        let _ = self.signal_tx.try_send(FlushTestSignal::Cancel);
    }
}

/// Wait for a logger's flush-entry signal while releasing the GIL.
#[pyfunction]
pub(crate) fn wait_for_flush_entry_for_test(
    py: Python<'_>,
    logger: PyRef<'_, FemtoLogger>,
    timeout_ms: u64,
) -> bool {
    let logger_ref: &FemtoLogger = &logger;
    py.detach(|| logger_ref.flush_test_hooks.wait_for_entry(timeout_ms))
}

/// Cancel a logger's pending flush-entry test wait during cleanup.
#[pyfunction]
pub(crate) fn cancel_flush_wait_for_test(logger: PyRef<'_, FemtoLogger>) {
    logger.flush_test_hooks.cancel_wait();
}
