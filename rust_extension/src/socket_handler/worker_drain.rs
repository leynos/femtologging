//! Shutdown-drain helpers for the socket worker.
//!
//! These functions work on the command queue alone and never touch the
//! transport, so the worker can discard a backlog without writing to a
//! collector that has stopped reading.

use std::io;

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use log::warn;

use crate::handler::report_drops_then_acknowledge;

use super::SocketCommand;

/// Throw away everything left in the queue, without touching the network.
///
/// Returns how many records were discarded. A flush sender is answered
/// immediately with a failure, because its marker was discarded rather than
/// written; shutdown senders are saved for the worker's final action, after it
/// has finished all remaining cleanup.
pub(super) fn discard_rest(
    rx: &Receiver<SocketCommand>,
    shutdown_acks: &mut Vec<Sender<()>>,
) -> u64 {
    let mut discarded = 0_u64;
    loop {
        match rx.try_recv() {
            Ok(SocketCommand::Record(_)) => discarded += 1,
            Ok(SocketCommand::Flush(ack, pending_drops)) => {
                report_drops_then_acknowledge(
                    pending_drops,
                    |count| {
                        warn!("FemtoSocketHandler dropped {count} records in the last interval")
                    },
                    || {
                        // The backlog is being discarded rather than written,
                        // so this marker was not flushed. Report that rather
                        // than confirm a flush that never touched the socket.
                        let _ = ack.send(Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            "socket worker discarded the flush marker during shutdown",
                        )));
                    },
                );
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
