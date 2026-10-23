//! Tests for the concurrency seam in both configurations.
//!
//! The ordinary tests run under `make test`. The `loom_` tests exist only
//! under `--cfg loom`, where they check the hand-built channel inside a Loom
//! model; the heavy lane's models then rely on it.

#[cfg(loom)]
mod loom_arm {
    //! The seam's Loom arm: the hand-built bounded channel.

    // Named rather than globbed from the seam: this module is the only place
    // that exercises the Loom arm, so its inputs are worth stating. The
    // ordinary arm has no tests left here, and a file-level glob would
    // otherwise have to exist for the sake of five names.
    use crate::sync::{TrySendError, bounded, spawn};

    /// A full channel refuses `try_send` and hands the message back.
    #[test]
    fn loom_try_send_refuses_a_full_channel() {
        loom::model(|| {
            let (tx, _rx) = bounded::<u8>(1);
            tx.try_send(1)
                .expect("an empty channel accepts one message");
            assert_eq!(tx.try_send(2), Err(TrySendError::Full(2)));
        });
    }

    /// Queued messages survive the last sender and are delivered before the
    /// receiver reports disconnection, in every interleaving.
    #[test]
    fn loom_receiver_drains_before_disconnecting() {
        loom::model(|| {
            let (tx, rx) = bounded::<u8>(2);
            let producer = spawn(move || {
                tx.send(1).expect("the receiver is alive");
                tx.send(2).expect("the receiver is alive");
            });
            let received: Vec<u8> = rx.into_iter().collect();
            producer.join().expect("the producer does not panic");
            assert_eq!(received, [1, 2]);
        });
    }

    /// A blocked sender is released by a receive, so a bounded channel of
    /// one carries more messages than its capacity without deadlocking.
    #[test]
    fn loom_send_waits_for_room() {
        loom::model(|| {
            let (tx, rx) = bounded::<u8>(1);
            let producer = spawn(move || {
                for message in 0..3 {
                    tx.send(message).expect("the receiver is alive");
                }
            });
            let received: Vec<u8> = rx.into_iter().collect();
            producer.join().expect("the producer does not panic");
            assert_eq!(received, [0, 1, 2]);
        });
    }

    /// A sender blocks while the channel is full: no interleaving lets a
    /// second message into a channel of one before the first is received.
    #[test]
    fn loom_send_blocks_while_full() {
        use loom::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };

        loom::model(|| {
            let (tx, rx) = bounded::<u8>(1);
            let is_done = Arc::new(AtomicBool::new(false));
            let producer = {
                let is_done = Arc::clone(&is_done);
                spawn(move || {
                    tx.send(0).expect("the receiver is alive");
                    tx.send(1).expect("the receiver is alive");
                    is_done.store(true, Ordering::SeqCst);
                })
            };
            assert!(
                !is_done.load(Ordering::SeqCst),
                "a second send completed while the channel was full"
            );
            let received: Vec<u8> = rx.into_iter().collect();
            producer.join().expect("the producer does not panic");
            assert_eq!(received, [0, 1]);
        });
    }

    /// A receiver wakes for a message another thread sends after it began
    /// waiting, so a worker blocked on an empty queue still sees work arrive.
    #[test]
    fn loom_recv_waits_for_another_thread() {
        loom::model(|| {
            let (tx, rx) = bounded::<u8>(1);
            let producer = spawn(move || {
                tx.send(9).expect("the receiver is alive");
            });
            assert_eq!(rx.recv(), Ok(9));
            producer.join().expect("the producer does not panic");
        });
    }
}
