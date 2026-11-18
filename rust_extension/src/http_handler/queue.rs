//! The HTTP worker's command channel: its commands, and both ends of it.
//!
//! The worker in `worker.rs` owns the network; this module owns the queue in
//! front of it. The split is where the two lifecycles meet: the worker's job is
//! to send what it is handed, while this module's is to hand out and dispose of
//! records on a channel whose capacity and destruction are the handler's
//! business rather than the worker's.
//!
//! Discarding is the part that needs saying out loud. A shutdown deadline that
//! arrives with records still queued costs those records, and the count is
//! warned about rather than swallowed, because a shutdown that quietly loses
//! logs is the failure this handler's deadline exists to make visible.

use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError, bounded};
use log::warn;

use crate::{
    handler::HandlerError, log_record::FemtoLogRecord, rate_limited_warner::RateLimitedWarner,
};

/// Commands processed by the worker thread.
#[derive(Debug)]
#[expect(
    clippy::large_enum_variant,
    reason = "Record variant is the hot path; wrapping in Box would add indirection for no benefit"
)]
pub enum HTTPCommand {
    /// Carry one record for HTTP serialization and delivery.
    Record(FemtoLogRecord),
    /// Flush earlier commands and acknowledge completion.
    Flush(Sender<()>),
    /// Begin shutdown and acknowledge after worker cleanup.
    Shutdown(Sender<()>),
}

/// Record a drop against `warner` and log the running count if it is due.
///
/// A shared helper rather than a method on either side of the split, because
/// the warner is the handler's and both the enqueue path and the worker's
/// error paths count into it.
pub(crate) fn warn_drops(warner: &RateLimitedWarner, log: impl FnMut(u64)) {
    warner.record_drop();
    warner.warn_if_due(log);
}

/// Report worker-owned drops without waiting for the warning interval.
///
/// Drops the worker discovers — serialization failures, retry exhaustion,
/// permanent errors — are recorded by the worker rather than by the handler, so
/// only the worker can report them. A suppressed interval would otherwise keep
/// the count until the next due warning, which on a worker that is exiting
/// never comes.
pub(crate) fn flush_drops(warner: &RateLimitedWarner) {
    warner.flush(|count| {
        log::warn!("FemtoHTTPHandler dropped {count} records since the last report");
    });
}

/// Record a drop and report the cumulative count, naming this drop's cause as
/// the one that triggered the report.
///
/// One warner counts drops from several causes — the handler's sees queue-full,
/// disconnection and closed refusals, the worker's sees serialization,
/// connection and write failures — so a due warning can carry drops from
/// several causes. Saying the count happened for this call's reason would
/// misattribute the earlier ones, and nothing here can know which of them was
/// most recent: another thread may record a drop between this one and the
/// report. The wording therefore names only the cause whose refusal asked for
/// the warning.
pub(crate) fn warn_drops_naming_cause(warner: &RateLimitedWarner, cause: &str) {
    warn_drops(warner, |count| warn_cumulative_drops(count, cause));
}

/// Report the pending cumulative count now, naming the triggering cause.
///
/// The flush equivalent of [`warn_drops_naming_cause`], for paths that cannot
/// wait for the interval and whose count may still hold drops from earlier
/// causes.
pub(crate) fn flush_drops_naming_cause(warner: &RateLimitedWarner, cause: &str) {
    warner.flush(|count| warn_cumulative_drops(count, cause));
}

/// Render the one cumulative drop warning the naming helpers emit.
///
/// Visible to the handler's tests so a controlled race between two causes can
/// assert on the message the production helper would produce for its winner.
pub(crate) fn warn_cumulative_drops(count: u64, cause: &str) {
    warn!(
        concat!(
            "FemtoHTTPHandler dropped {} records since the last report; ",
            "triggering cause: {}"
        ),
        count, cause
    );
}

/// Throw away everything left in the queue, without touching the network.
///
/// Returns how many records were discarded. Flush senders receive their
/// acknowledgement immediately; shutdown senders are saved for the worker's
/// final action, after it has finished all remaining cleanup.
pub(crate) fn discard_rest(rx: &Receiver<HTTPCommand>, shutdown_acks: &mut Vec<Sender<()>>) -> u64 {
    let mut discarded = 0_u64;
    loop {
        match rx.try_recv() {
            Ok(HTTPCommand::Record(_)) => discarded += 1,
            Ok(HTTPCommand::Flush(ack)) => {
                let _ = ack.send(());
            }
            Ok(HTTPCommand::Shutdown(ack)) => shutdown_acks.push(ack),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return discarded,
        }
    }
}

/// Say how many records the shutdown deadline cost.
///
/// Deliberately not rate-limited through the warner: this fires at most once
/// per handler, and a count of the records a shutdown abandoned is the kind of
/// thing a caller needs to see whether or not a warning interval has elapsed.
pub(crate) fn warn_discarded(discarded: u64) {
    if discarded == 0 {
        return;
    }
    warn!("FemtoHTTPHandler discarded {discarded} queued records at its shutdown deadline");
}

/// Enqueues a log record for transmission by the HTTP worker.
///
/// This is a non-blocking operation. If the queue is full, the record is
/// dropped and a rate-limited warning is emitted.
///
/// # Arguments
///
/// * `tx` - Sender channel to the HTTP worker thread
/// * `record` - The log record to enqueue
/// * `warner` - Rate-limited warner for drop notifications
///
/// # Errors
///
/// * [`HandlerError::QueueFull`] - The queue is at capacity; record was dropped
/// * [`HandlerError::Closed`] - The worker has shut down; record was dropped
pub fn enqueue_record(
    tx: &Sender<HTTPCommand>,
    record: FemtoLogRecord,
    warner: &RateLimitedWarner,
) -> Result<(), HandlerError> {
    match tx.try_send(HTTPCommand::Record(record)) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(_)) => {
            warn_drops_naming_cause(warner, "queue full");
            Err(HandlerError::QueueFull)
        }
        Err(TrySendError::Disconnected(_)) => {
            warn_drops_naming_cause(warner, "worker disconnected");
            Err(HandlerError::Closed)
        }
    }
}

/// Sends a flush command to the HTTP worker and waits for acknowledgment.
///
/// Uses a deadline-based approach to ensure the total wait time does not
/// exceed `timeout`, even if the send operation consumes part of the budget.
///
/// # Arguments
///
/// * `tx` - Sender channel to the HTTP worker thread
/// * `timeout` - Maximum time to wait for both sending and receiving the ack
///
/// # Returns
///
/// `true` if the flush was acknowledged within the timeout, `false` otherwise.
pub fn flush_queue(tx: &Sender<HTTPCommand>, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let (ack_tx, ack_rx) = bounded(1);
    if tx
        .send_timeout(HTTPCommand::Flush(ack_tx), timeout)
        .is_err()
    {
        return false;
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    ack_rx.recv_timeout(remaining).is_ok()
}
