//! Barrier insertion and drain waits for one shard.
//!
//! A shard answers two kinds of wait. A plain [`Job::Barrier`] says that every
//! job queued ahead of it has been handled, and a [`Job::Flush`] additionally
//! carries a handler snapshot that the worker flushes before acknowledging.
//! Both are bounded: the yes-or-no drain keeps [`BARRIER_TIMEOUT`], while a
//! flush supplies its own absolute deadline so the whole barrier is charged to
//! one budget rather than one per stage.
//!
//! [`Job::Barrier`]: super::Job::Barrier
//! [`Job::Flush`]: super::Job::Flush

use std::sync::Arc;
use std::time::Instant;

use crate::handler::FemtoHandlerTrait;
use crate::logger::flush_clock::FlushDeadline;
use crate::logger::flush_diagnostics::FlushFailureStage;
use crate::sync::{RecvTimeoutError, SendTimeoutError, Sender, bounded, recv_timeout};

use super::{BARRIER_TIMEOUT, DrainOutcome, Job, Shard};

impl Shard {
    /// Wait for the shard to finish the work queued ahead of this call.
    ///
    /// Returns `false` when the shard is gone or does not answer within
    /// [`BARRIER_TIMEOUT`], which the caller reports as a failed flush or an
    /// incomplete drain.
    ///
    /// The barrier is queued with `send_timeout` rather than `try_send`: a
    /// shard at the end of a burst has a full queue but a working worker, and
    /// refusing the barrier then would report a drain that did happen as a
    /// flush that failed. Waiting for room makes the answer mean "everything
    /// ahead of this call was handled", which is what a flush promises.
    ///
    /// Both waits share one deadline, so a call is bounded by
    /// [`BARRIER_TIMEOUT`] in total rather than by twice it. Queueing the
    /// barrier and hearing its answer are two halves of one wait, and a
    /// caller that was promised a two-second drain would otherwise block for
    /// four when the queue is full and the worker is slow.
    ///
    /// Only the non-Loom shutdown path and its unit tests ask the yes-or-no
    /// question; the flush path needs the cause, so a Loom build would
    /// otherwise carry this method unused.
    #[cfg(not(loom))]
    pub(crate) fn wait_for_drain(&self) -> bool {
        matches!(self.wait_for_drain_outcome(), DrainOutcome::Drained)
    }

    /// Wait for the shard to finish the work queued ahead of this call,
    /// reporting *why* the wait ended.
    ///
    /// `wait_for_drain` answers only success or failure, which is all a
    /// barrier's callers need to decide whether to keep waiting. The flush path
    /// also counts its failures by cause, so it needs the close-or-timeout
    /// distinction the channel error variants already carry.
    ///
    /// The two waits share one deadline, exactly as `wait_for_drain`
    /// documents: queueing the barrier and hearing its answer are two halves of
    /// a single [`BARRIER_TIMEOUT`] budget.
    pub(crate) fn wait_for_drain_outcome(&self) -> DrainOutcome {
        let deadline = Instant::now() + BARRIER_TIMEOUT;
        let (ack, acked) = bounded::<()>(1);
        match self
            .queue
            .send_timeout(Job::Barrier { ack }, BARRIER_TIMEOUT)
        {
            Ok(()) => {}
            Err(SendTimeoutError::Disconnected(_)) => return DrainOutcome::QueueClosed,
            Err(SendTimeoutError::Timeout(_)) => return DrainOutcome::TimedOut,
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        match recv_timeout(&acked, remaining) {
            Ok(()) => DrainOutcome::Drained,
            // The acknowledgement channel can only disconnect when its sender
            // is dropped without answering, and the worker owns that sender
            // for as long as it lives. A disconnect therefore means the worker
            // is gone, not that it was slow.
            Err(RecvTimeoutError::Disconnected) => DrainOutcome::QueueClosed,
            Err(RecvTimeoutError::Timeout) => DrainOutcome::TimedOut,
        }
    }

    /// Wait for a drain barrier using a deadline created by the caller.
    ///
    /// Unlike [`Self::wait_for_drain_outcome`], which sets its own budget, this
    /// shares one deadline with the rest of a flush barrier and names the stage
    /// that expired so the diagnostics can count it.
    pub(crate) fn wait_for_drain_until(
        &self,
        deadline: &FlushDeadline,
    ) -> Result<(), FlushFailureStage> {
        if deadline.is_expired() {
            return Err(FlushFailureStage::DeadlineExpiredBeforeMarker);
        }
        let (ack, acked) = bounded::<()>(1);
        match self
            .queue
            .send_timeout(Job::Barrier { ack }, deadline.remaining())
        {
            Ok(()) => {}
            Err(SendTimeoutError::Timeout(_)) => {
                return Err(FlushFailureStage::MarkerEnqueueTimedOut);
            }
            Err(SendTimeoutError::Disconnected(_)) => {
                return Err(FlushFailureStage::MarkerEnqueueDisconnected);
            }
        }
        match recv_timeout(&acked, deadline.remaining()) {
            Ok(()) => Ok(()),
            Err(RecvTimeoutError::Timeout) => Err(FlushFailureStage::AcknowledgementTimedOut),
            Err(RecvTimeoutError::Disconnected) => Err(FlushFailureStage::WorkerExited),
        }
    }

    /// Enqueue a handler marker behind prior work and wait until `deadline`.
    ///
    /// The marker carries only channels while the bounded queue is being
    /// admitted, so a saturated queue never holds the handler registry lock or
    /// owns a handler snapshot that might run a custom destructor on this
    /// caller. After admission, `prepare_snapshot` captures and transfers the
    /// snapshot while the registry is read-locked. The worker waits at `start`
    /// until that lock is released before invoking callbacks. The queue remains
    /// the only task bound, so concurrent timeouts cannot create extra threads
    /// or unbounded queued work.
    pub(crate) fn flush_handlers_until<F>(
        &self,
        deadline: &FlushDeadline,
        prepare_snapshot: F,
    ) -> Result<(), FlushFailureStage>
    where
        F: FnOnce(Sender<Vec<Arc<dyn FemtoHandlerTrait>>>) -> Result<(), FlushFailureStage>,
    {
        let (handlers_tx, handlers_rx) = bounded(1);
        let (start_tx, start_rx) = bounded(1);
        let (ack_tx, ack_rx) = bounded(1);
        let job = Job::Flush {
            handlers: handlers_rx,
            deadline: deadline.clone(),
            start: start_rx,
            ack: ack_tx,
            enqueued_at: deadline.now(),
        };

        if deadline.is_expired() {
            drop(job);
            return Err(FlushFailureStage::DeadlineExpiredBeforeMarker);
        }
        match self.queue.send_timeout(job, deadline.remaining()) {
            Ok(()) => {}
            Err(SendTimeoutError::Timeout(_)) => {
                return Err(FlushFailureStage::MarkerEnqueueTimedOut);
            }
            Err(SendTimeoutError::Disconnected(_)) => {
                return Err(FlushFailureStage::MarkerEnqueueDisconnected);
            }
        }

        if let Err(stage) = prepare_snapshot(handlers_tx) {
            drop(start_tx);
            return Err(stage);
        }
        if deadline.is_expired() {
            drop(start_tx);
            return Err(FlushFailureStage::WorkerStartTimedOut);
        }

        if start_tx.try_send(()).is_err() {
            return Err(FlushFailureStage::WorkerStartDisconnected);
        }
        match recv_timeout(&ack_rx, deadline.remaining()) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => Err(FlushFailureStage::AcknowledgementTimedOut),
            Err(RecvTimeoutError::Disconnected) => Err(FlushFailureStage::WorkerExited),
        }
    }
}
