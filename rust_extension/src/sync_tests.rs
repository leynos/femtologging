//! Tests for the concurrency seam in both configurations.
//!
//! The ordinary tests run under `make test`. The `loom_` tests exist only
//! under `--cfg loom`, where they check the hand-built channel inside a Loom
//! model; the heavy lane's models then rely on it.

use super::*;

#[cfg(not(loom))]
mod ordinary {
    //! The seam's ordinary arm: `crossbeam_channel` and its `select!`.

    use rstest::rstest;

    use super::*;

    /// `recv_either` reports whichever receiver holds a message.
    #[rstest]
    #[case::first(true)]
    #[case::second(false)]
    fn recv_either_takes_the_ready_receiver(#[case] is_first_ready: bool) {
        let (first_tx, first_rx) = bounded::<u8>(1);
        let (second_tx, second_rx) = bounded::<u8>(1);
        let sender = if is_first_ready {
            &first_tx
        } else {
            &second_tx
        };
        sender
            .send(7)
            .expect("an empty channel accepts one message");

        let expected = if is_first_ready {
            Either::First(Ok(7))
        } else {
            Either::Second(Ok(7))
        };
        assert_eq!(recv_either(&first_rx, &second_rx), expected);
    }

    /// A dropped sender makes its receiver ready, so a worker waiting on a
    /// shutdown channel notices the logger going away.
    #[test]
    fn recv_either_treats_disconnection_as_ready() {
        let (first_tx, first_rx) = bounded::<u8>(1);
        let (_second_tx, second_rx) = bounded::<u8>(1);
        drop(first_tx);

        assert_eq!(
            recv_either(&first_rx, &second_rx),
            Either::First(Err(RecvError))
        );
    }
}

#[cfg(loom)]
mod loom_arm {
    //! The seam's Loom arm: the hand-built bounded channel.

    use super::*;

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

    /// `recv_either` wakes for whichever channel another thread fills.
    #[test]
    fn loom_recv_either_waits_for_another_thread() {
        loom::model(|| {
            let (_first_tx, first_rx) = bounded::<u8>(1);
            let (second_tx, second_rx) = bounded::<u8>(1);
            let producer = spawn(move || {
                second_tx.send(9).expect("the receiver is alive");
            });
            assert_eq!(recv_either(&first_rx, &second_rx), Either::Second(Ok(9)));
            producer.join().expect("the producer does not panic");
        });
    }
}
