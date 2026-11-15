//! Flush requests for the file handler.
//!
//! A flush travels the same queue as a record, so its acknowledgement proves
//! the worker has handled everything buffered ahead of it. The enqueue and the
//! acknowledgement share one deadline but are reported separately: a request
//! that never reached the worker and a request the worker never answered point
//! at different faults.

use std::io;
use std::time::{Duration, Instant};

use crate::handler::{
    DEFAULT_FLUSH_TIMEOUT, FlushContext, QueueFlushFailure, capped_deadline, deadline_after,
    flush_command_until,
};
use crate::sync::{Receiver, RecvTimeoutError, SendTimeoutError, bounded};

use super::{FemtoFileHandler, FileCommand, FlushOutcome};

impl FemtoFileHandler {
    /// Flush the worker's buffer, waiting up to one second. Returns `true` only when the flush
    /// is confirmed in time; `false` if the handler is closed, unreachable or the flush fails.
    pub fn flush(&self) -> bool {
        matches!(self.flush_outcome(), FlushOutcome::Acknowledged)
    }

    /// Flush and report why the attempt finished.
    ///
    /// The Python binding records this outcome so an operator can tell a
    /// timed-out flush from a flush of an already-closed handler, which the
    /// public `bool` cannot express.
    pub(crate) fn flush_outcome(&self) -> FlushOutcome {
        let now = Instant::now();
        let Some(deadline) = deadline_after(now, DEFAULT_FLUSH_TIMEOUT) else {
            // The clock sits at the end of its representable range, so no
            // acknowledgement can be waited for.
            return FlushOutcome::AckTimedOut;
        };
        self.flush_outcome_at(deadline, &mut Instant::now)
    }

    /// Flush the worker, waiting no longer than the context's deadline.
    ///
    /// The wait also stops at [`DEFAULT_FLUSH_TIMEOUT`], so a bare `flush()`
    /// keeps the second it documents and a barrier flush cannot outlive the
    /// deadline it was given. Time is sampled from the context, so a barrier
    /// test controls this handler's deadline transition as well as the
    /// logger's.
    pub(crate) fn flush_by(&self, context: &FlushContext) -> bool {
        let Some(deadline) = context.deadline() else {
            return false;
        };
        matches!(
            self.flush_outcome_at(deadline, &mut || context.now()),
            FlushOutcome::Acknowledged
        )
    }

    /// Flush against `deadline`, sampling each queue wait from `now`.
    ///
    /// Production passes [`Instant::now`] here; tests pass a scripted source,
    /// so a stalled worker queue can be shown to consume exactly the budget
    /// the caller supplied rather than a fresh copy for each wait.
    #[cfg(test)]
    pub(super) fn flush_by_at(&self, deadline: Instant, now: &mut dyn FnMut() -> Instant) -> bool {
        matches!(
            self.flush_outcome_at(deadline, now),
            FlushOutcome::Acknowledged
        )
    }

    /// Classify one flush attempt bounded by `deadline`.
    ///
    /// The enqueue and the acknowledgement share one deadline but are reported
    /// separately: a request that never reached the worker and a request the
    /// worker answered too late point at different faults.
    fn flush_outcome_at(
        &self,
        deadline: Instant,
        now: &mut dyn FnMut() -> Instant,
    ) -> FlushOutcome {
        // Whichever comes first — the caller's deadline or this handler's own
        // budget — is the one instant both waits below share.
        let effective = capped_deadline(deadline, now(), DEFAULT_FLUSH_TIMEOUT);
        // Bind first: a guard in the `match` scrutinee lives through the send.
        let tx = self.tx.lock().clone();
        let Some(tx) = tx else {
            return FlushOutcome::Closed;
        };

        // `flush_command_until` reports only that admission failed, so the send
        // error is kept here to tell a dead worker from a full queue.
        let mut admission_failure = None;
        let outcome = flush_command_until(effective, now, |budget| {
            let (ack_tx, ack_rx) = bounded(1);
            match tx.send_timeout(FileCommand::Flush(ack_tx), budget) {
                Ok(()) => Some(move |budget| Some(Self::classify_acknowledgement(&ack_rx, budget))),
                Err(err) => {
                    admission_failure = Some(err);
                    None
                }
            }
        });
        match outcome {
            Ok(outcome) => outcome,
            Err(QueueFlushFailure::Enqueue) => match admission_failure {
                Some(SendTimeoutError::Disconnected(_)) => FlushOutcome::Disconnected,
                _ => FlushOutcome::EnqueueTimedOut,
            },
            // Either the closure's own receive timed out, or it returned an
            // answer the shared helper saw arrive after the deadline. Both are
            // a late acknowledgement from this handler's point of view.
            Err(QueueFlushFailure::Acknowledgement) => FlushOutcome::AckTimedOut,
        }
    }

    /// Classifies the worker's answer to a flush request.
    ///
    /// `budget` is what remains of the shared deadline, not a fresh timeout, so
    /// a slow enqueue leaves the acknowledgement less to spend.
    fn classify_acknowledgement(
        ack_rx: &Receiver<io::Result<()>>,
        budget: Duration,
    ) -> FlushOutcome {
        match ack_rx.recv_timeout(budget) {
            Ok(Ok(())) => FlushOutcome::Acknowledged,
            Ok(Err(_)) => FlushOutcome::Failed,
            Err(RecvTimeoutError::Timeout) => FlushOutcome::AckTimedOut,
            // The worker dropped the acknowledgement sender without answering,
            // which means it died between accepting the request and replying.
            Err(RecvTimeoutError::Disconnected) => FlushOutcome::Disconnected,
        }
    }
}
