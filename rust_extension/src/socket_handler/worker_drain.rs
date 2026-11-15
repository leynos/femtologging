//! Shutdown-drain helpers for the socket worker.
//!
//! These functions work on the command queue alone and never touch the
//! transport, so the worker can discard a backlog without writing to a
//! collector that has stopped reading.

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use log::warn;

use super::SocketCommand;

/// Throw away everything left in the queue, without touching the network.
///
/// Returns how many records were discarded. Flush senders receive their
/// acknowledgement immediately; shutdown senders are saved for the worker's
/// final action, after it has finished all remaining cleanup.
pub(super) fn discard_rest(
    rx: &Receiver<SocketCommand>,
    shutdown_acks: &mut Vec<Sender<()>>,
) -> u64 {
    let mut discarded = 0_u64;
    loop {
        match rx.try_recv() {
            Ok(SocketCommand::Record(_)) => discarded += 1,
            Ok(SocketCommand::Flush(ack, pending_drops)) => {
                let _ = ack.send(());
                if let Some(count) = pending_drops {
                    warn!("FemtoSocketHandler dropped {count} records in the last interval");
                }
            }
            Ok(SocketCommand::Shutdown(ack)) => shutdown_acks.push(ack),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return discarded,
        }
    }
}

/// Say how many records the shutdown deadline cost, once.
///
/// Deliberately not rate-limited through the warner: this fires at most once
/// per handler, and a count of the records a shutdown abandoned is the kind of
/// thing a caller needs to see whether or not a warning interval has elapsed.
pub(super) fn warn_discarded(discarded: u64) {
    if discarded == 0 {
        return;
    }
    warn!("FemtoSocketHandler discarded {discarded} queued records at its shutdown deadline");
}
