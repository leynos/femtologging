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

use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError};
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
    /// Flush earlier commands, then acknowledge with any drop count the caller
    /// took from the warner.
    Flush(Sender<()>, Option<u64>),
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
            Ok(HTTPCommand::Flush(ack, pending_drops)) => {
                let _ = ack.send(());
                if let Some(count) = pending_drops {
                    warn!("FemtoHTTPHandler dropped {count} records in the last interval");
                }
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
            warn_drops(warner, |count| {
                warn!("FemtoHTTPHandler queue full; dropped {count} records");
            });
            Err(HandlerError::QueueFull)
        }
        Err(TrySendError::Disconnected(_)) => {
            warn_drops(warner, |count| {
                warn!("FemtoHTTPHandler disconnected; dropped {count} records");
            });
            Err(HandlerError::Closed)
        }
    }
}
