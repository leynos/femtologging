//! The closed outcome vocabulary for one Python-facing handler wait.
//!
//! Each variant maps to exactly one counter and one stable string key. Keeping
//! the mapping here is what makes the exposed dictionary fixed-cardinality: a
//! new outcome is one variant plus one arm, and the key-count test fails until
//! the Python surface acknowledges it.

/// Number of outcomes a handler wait can report.
pub(crate) const OUTCOME_COUNT: usize = 16;

/// Outcome of one Python-facing handler wait.
///
/// The variants are per operation so the counter key can be derived without
/// consulting the operation name; the closed set is what keeps the exposed
/// dictionary fixed-cardinality.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WaitOutcome {
    /// `handle()` enqueued the record.
    HandleAccepted,
    /// `handle()` found the queue full under the drop policy.
    HandleQueueFull,
    /// `handle()` reached its timeout without enqueueing.
    HandleTimedOut,
    /// `handle()` found the handler already closed.
    HandleClosed,
    /// `handle()` found the worker's channel gone while the handler was open.
    HandleDisconnected,
    /// `handle()` failed for a reason the handler cannot classify further.
    HandleFailed,
    /// `flush()` received the worker's acknowledgement.
    FlushAcknowledged,
    /// `flush()` could not enqueue the request before its deadline.
    FlushEnqueueTimedOut,
    /// `flush()` enqueued the request but the worker did not acknowledge.
    FlushAckTimedOut,
    /// `flush()` found the handler already closed.
    FlushClosed,
    /// `flush()` found the worker's channel gone while the handler was open.
    FlushDisconnected,
    /// The worker answered the flush with an I/O failure.
    FlushFailed,
    /// `close()` observed the worker stop cleanly.
    CloseCompleted,
    /// `close()` reached its one-second bound without the worker stopping.
    CloseTimedOut,
    /// `close()` observed a worker that had panicked.
    ClosePanicked,
    /// `close()` observed completion but could not establish a verdict.
    CloseUnknown,
}

impl WaitOutcome {
    /// Every outcome, in counter-index order.
    pub(super) const ALL: [Self; OUTCOME_COUNT] = [
        Self::HandleAccepted,
        Self::HandleQueueFull,
        Self::HandleTimedOut,
        Self::HandleClosed,
        Self::HandleDisconnected,
        Self::HandleFailed,
        Self::FlushAcknowledged,
        Self::FlushEnqueueTimedOut,
        Self::FlushAckTimedOut,
        Self::FlushClosed,
        Self::FlushDisconnected,
        Self::FlushFailed,
        Self::CloseCompleted,
        Self::CloseTimedOut,
        Self::ClosePanicked,
        Self::CloseUnknown,
    ];

    /// Stable key used in the Python dictionary and the tracing event.
    ///
    /// The operation prefix is part of the key so a single flat dictionary
    /// cannot confuse a `handle()` outcome with a `flush()` outcome.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::HandleAccepted => "handle_accepted",
            Self::HandleQueueFull => "handle_queue_full",
            Self::HandleTimedOut => "handle_timed_out",
            Self::HandleClosed => "handle_closed",
            Self::HandleDisconnected => "handle_disconnected",
            Self::HandleFailed => "handle_failed",
            Self::FlushAcknowledged => "flush_acknowledged",
            Self::FlushEnqueueTimedOut => "flush_enqueue_timed_out",
            Self::FlushAckTimedOut => "flush_ack_timed_out",
            Self::FlushClosed => "flush_closed",
            Self::FlushDisconnected => "flush_disconnected",
            Self::FlushFailed => "flush_failed",
            Self::CloseCompleted => "close_completed",
            Self::CloseTimedOut => "close_timed_out",
            Self::ClosePanicked => "close_panicked",
            Self::CloseUnknown => "close_unknown",
        }
    }

    /// Position of the outcome in the counter array.
    pub(super) fn index(self) -> usize {
        self as usize
    }
}
