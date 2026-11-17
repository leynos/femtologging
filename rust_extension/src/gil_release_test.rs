//! Test-only signal raised where a Python-facing wait releases the GIL.
//!
//! The GIL regression tests for `flush_handlers()`, `flush()` and `close()`
//! have to know that the call under test has reached the point where it gives
//! the interpreter up. Being inside the call is not enough: `flush_handlers()`
//! detaches around the whole call, so a blocked formatter released anywhere in
//! that window could be released in front of the native wait, and a wait that
//! held the GIL would then find the record already flushed. A test-only clock
//! or a frame observation cannot tell those two schedules apart.
//!
//! Each detach site therefore raises this gate as its last action while the
//! GIL is still held, immediately before `Python::detach`. A waiter that
//! observes the gate has, by construction, observed a thread that has since
//! released the GIL: it could not have run otherwise. In a build where a
//! detach was removed the same thread keeps the GIL through the wait, so no
//! waiter observes the gate and the test fails instead of passing by timing.
//!
//! The gate is process-wide because the waits are: each Python test that uses
//! it runs in a fresh interpreter with a single handler, so a signal cannot
//! belong to another test's call.

use std::sync::OnceLock;
use std::time::Duration;

use pyo3::prelude::*;

use crate::sync::{Receiver, Sender, bounded};

/// One-slot channel that reports GIL releases to a waiting test.
struct GilReleaseGate {
    /// Sends one notification per wait that releases the GIL.
    signal_tx: Sender<()>,
    /// Receives those notifications, or a timeout when none arrives.
    signal_rx: Receiver<()>,
}

impl GilReleaseGate {
    /// Create a gate whose buffer holds one unconsumed notification.
    fn new() -> Self {
        let (signal_tx, signal_rx) = bounded(1);
        Self {
            signal_tx,
            signal_rx,
        }
    }

    /// Report that the calling thread is about to wait with the GIL released.
    ///
    /// A second signal while one is still unconsumed is dropped rather than
    /// waited for: the gate reports that at least one release happened, and a
    /// flush must never block on a test.
    fn signal(&self) {
        let _ = self.signal_tx.try_send(());
    }

    /// Discard notifications from earlier calls before a test arms its gate.
    fn drain(&self) {
        while self.signal_rx.try_recv().is_ok() {}
    }

    /// Wait for the next release for at most *timeout_ms*, reporting whether
    /// one arrived.
    fn wait_for_release(&self, timeout_ms: u64) -> bool {
        self.signal_rx
            .recv_timeout(Duration::from_millis(timeout_ms))
            .is_ok()
    }
}

/// The gate every detach site reports to.
fn process_gate() -> &'static GilReleaseGate {
    static GATE: OnceLock<GilReleaseGate> = OnceLock::new();
    GATE.get_or_init(GilReleaseGate::new)
}

/// Report that the calling thread is about to wait with the GIL released.
///
/// The call site must be the last statement before `Python::detach`, so a
/// waiter can rely on the release happening after the observation.
pub(crate) fn signal_gil_release() {
    process_gate().signal();
}

/// Wait for a GIL release, detaching the interpreter while it waits.
///
/// Detaching is what makes this usable: the waiter runs while the call under
/// test holds no GIL, and holding one here would stop that call from ever
/// reaching its wait.
#[pyfunction]
pub(crate) fn wait_for_gil_release_for_test(py: Python<'_>, timeout_ms: u64) -> bool {
    py.detach(|| process_gate().wait_for_release(timeout_ms))
}

/// Discard GIL-release signals from earlier calls before a test arms its gate.
#[pyfunction]
pub(crate) fn drain_gil_release_signals_for_test() {
    process_gate().drain();
}

#[cfg(test)]
mod tests {
    //! Checks that a gate reports a release once and can be re-armed.

    use super::GilReleaseGate;

    #[test]
    fn draining_discards_a_stale_signal() {
        let gate = GilReleaseGate::new();
        gate.signal();
        gate.drain();

        assert!(!gate.wait_for_release(1));
    }

    #[test]
    fn a_signal_is_reported_to_the_waiter() {
        let gate = GilReleaseGate::new();
        gate.signal();

        assert!(gate.wait_for_release(1_000));
    }
}
