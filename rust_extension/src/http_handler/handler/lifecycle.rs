//! Deadline-bounded shutdown for the HTTP handler.
//!
//! These methods live apart from construction and flushing because they answer
//! a different question: how long the worker may take to stop, and what the
//! handler does when that budget runs out. They need the handler's private
//! fields, so they sit in a child module rather than a sibling one.

#[cfg(test)]
use std::sync::Arc;
use std::time::Instant;

use crate::handler::deadline_after;
#[cfg(test)]
use crate::shutdown_signal::ShutdownSignal;

use super::super::queue::HTTPCommand;
use super::FemtoHTTPHandler;

impl FemtoHTTPHandler {
    /// Ask the worker to stop, and report whether joining it is now bounded.
    ///
    /// The distinction the return value carries is the whole point: every
    /// outcome except a timeout means the worker has left, or is leaving, its
    /// command loop, so the join that follows is short. An earlier version
    /// discarded this with `let _ =`, which is what let an unbounded join
    /// follow a bounded wait.
    ///
    /// One deadline covers both halves. The command channel is bounded, so
    /// when it is full the send itself blocks until the worker next reads it,
    /// which a worker stuck mid-request does not do; a send outside the budget
    /// would put that whole wait back in front of the bounded receive.
    ///
    /// The signal is raised before the send, so it is the signal that wakes
    /// the worker that may be sleeping rather than the command that queues
    /// behind a full channel. A send that times out is not, on its own,
    /// evidence of anything the worker needs telling: the signal has already
    /// told it to wind down.
    pub(super) fn request_shutdown(&mut self) -> ShutdownOutcome {
        let Some(tx) = self.tx.take() else {
            // Already closed. There is no worker left to wait for.
            return ShutdownOutcome::Finished;
        };
        let Some(deadline) = deadline_after(Instant::now(), self.flush_timeout) else {
            // An unrepresentable deadline cannot be waited out, so the shutdown
            // fails closed rather than blocking on an unbounded join.
            return ShutdownOutcome::TimedOut;
        };
        if let Some(signal) = self.signal.as_ref() {
            signal.begin_shutdown(deadline);
        }
        let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
        match tx.send_deadline(HTTPCommand::Shutdown(ack_tx), deadline) {
            Ok(()) => {}
            // The worker has dropped its receiver, so it has already left the
            // loop; joining it returns at once and may report a panic.
            Err(crossbeam_channel::SendTimeoutError::Disconnected(_)) => {
                return ShutdownOutcome::Finished;
            }
            // The queue stayed full for the whole budget: the worker is not
            // reading commands, so it could not have acknowledged either.
            Err(crossbeam_channel::SendTimeoutError::Timeout(_)) => {
                return ShutdownOutcome::TimedOut;
            }
        }
        // Give up the last sender now the command is queued. The signal was
        // raised first, so the worker can leave between that and this send and
        // never read the command - and the queued command owns the only real
        // acknowledgement sender, so while this one lived the wait below could
        // neither receive nor see a disconnect. It then ran out the whole
        // budget and reported a worker that had already finished as abandoned.
        //
        // Dropping it cannot lose a live acknowledgement: a worker still in
        // its loop reads queued commands after every sender is gone, and one
        // that has left drops the queue, which drops the unread command and
        // disconnects the channel the wait below already treats as "left".
        drop(tx);
        match ack_rx.recv_deadline(deadline) {
            Ok(()) => ShutdownOutcome::Finished,
            // The worker dropped the acknowledgement channel without using it,
            // which it can only do on its way out.
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => ShutdownOutcome::Finished,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => ShutdownOutcome::TimedOut,
        }
    }

    /// Clone the worker signal so tests can observe that shutdown was
    /// published before the worker's wait is interrupted.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let signal = handler.signal_for_test();
    /// assert!(!signal.is_shutting_down());
    /// ```
    #[cfg(test)]
    pub(crate) fn signal_for_test(&self) -> Arc<ShutdownSignal> {
        Arc::clone(self.signal.as_ref().expect("an open handler has a signal"))
    }

    /// Join the worker after its acknowledgement or channel disconnection.
    ///
    /// Example: an acknowledged shutdown takes and joins the worker handle.
    pub(super) fn join_worker(&mut self) {
        self.signal.take();
        let Some(handle) = self.handle.lock().take() else {
            return;
        };
        if handle.join().is_err() {
            log::warn!("FemtoHTTPHandler: worker thread panicked");
        }
    }

    /// Detach the worker when shutdown could not be confirmed.
    ///
    /// The handle is taken so nothing joins it later, in particular the `Drop`
    /// that may follow this call. The thread itself ends on its own once its
    /// in-flight request finishes, because the signal now says nothing further
    /// may be started and its command channel is already disconnected.
    ///
    /// The signal is aborted rather than simply dropped, and that is the one
    /// thing this call cannot leave to the thread's own good sense: the worker
    /// is the only holder of the other reference, so a signal that stayed in
    /// the draining phase would keep telling it there was budget left until
    /// the drain deadline it was already past.
    pub(super) fn abandon_worker(&mut self) {
        if let Some(signal) = self.signal.take() {
            signal.abort();
        }
        log::warn!(
            "FemtoHTTPHandler: could not confirm worker shutdown within {:?}; \
             abandoning it rather than blocking the caller",
            self.flush_timeout
        );
        self.handle.lock().take();
    }
}

/// What asking the worker to stop established about joining it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ShutdownOutcome {
    /// The worker acknowledged, or had already left its loop. The join is
    /// bounded and worth doing, because it is also what surfaces a panic.
    Finished,
    /// The command could not be queued, no acknowledgement arrived within
    /// `flush_timeout`, or that timeout could not be represented as a deadline.
    /// The worker is busy on something whose length `flush_timeout` does not
    /// describe, or the caller cannot safely wait for it.
    TimedOut,
}
