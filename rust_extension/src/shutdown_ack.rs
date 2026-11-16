//! Final shutdown acknowledgements shared by handler worker threads.
//!
//! Both the HTTP and socket workers hold their shutdown acknowledgements until
//! every queued request and all cleanup have finished. The protocol is about
//! crossbeam channels rather than either transport, so it lives here instead of
//! being repeated in each worker.

use crossbeam_channel::Sender;

/// Send every queued shutdown acknowledgement, then the worker's own.
///
/// The final send is the worker's last action, so a caller waiting on either
/// acknowledgement knows all queued work has been attempted.
pub(crate) fn acknowledge_shutdowns(
    shutdown_acks: Vec<Sender<()>>,
    shutdown_ack: Option<Sender<()>>,
) {
    for ack in shutdown_acks {
        let _ = ack.send(());
    }

    if let Some(ack) = shutdown_ack {
        let _ = ack.send(());
    }
}
