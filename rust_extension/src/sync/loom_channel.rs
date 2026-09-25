//! A bounded multi-producer, multi-consumer channel built on Loom primitives.
//!
//! Loom's own `mpsc` channel is unbounded and has no `try_send`,
//! `send_timeout` or `recv_timeout`, and the workers use all three, so under
//! `--cfg loom` the seam supplies this channel in place of
//! `crossbeam_channel::bounded`. Its state lives behind one Loom mutex with one
//! condition variable, which is what lets Loom schedule every send, receive
//! and disconnection.
//!
//! It keeps the parts of `crossbeam_channel`'s contract the workers rely on:
//! a full channel refuses `try_send`, a channel whose senders are all dropped
//! drains and then reports disconnection, and a channel whose receivers are
//! all dropped refuses sends. A zero capacity, which `crossbeam_channel`
//! treats as a rendezvous, is raised to one; no handler builds a rendezvous
//! channel.
//!
//! The items are `pub` rather than `pub(crate)` because a `test-util` build
//! exposes the logger's sender through `FemtoLogger::clone_sender_for_test`;
//! the module itself stays private to the seam.

use std::collections::VecDeque;
use std::sync::PoisonError;
use std::time::Duration;

use loom::sync::{Arc, Condvar, Mutex, MutexGuard};

use crossbeam_channel::{
    RecvError, RecvTimeoutError, SendError, SendTimeoutError, TryRecvError, TrySendError,
};

/// The queue and the endpoint counts every operation reads.
struct State<T> {
    queue: VecDeque<T>,
    capacity: usize,
    senders: usize,
    receivers: usize,
}

/// State shared by every endpoint of one channel.
struct Shared<T> {
    state: Mutex<State<T>>,
    changed: Condvar,
}

impl<T> Shared<T> {
    /// Lock the channel state, ignoring poisoning as `parking_lot` would.
    fn lock(&self) -> MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Release `guard` and wait for another endpoint to change the state.
    fn wait<'a>(&self, guard: MutexGuard<'a, State<T>>) -> MutexGuard<'a, State<T>> {
        self.changed
            .wait(guard)
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// The sending half of a Loom-scheduled bounded channel.
pub struct Sender<T> {
    shared: Arc<Shared<T>>,
}

/// The receiving half of a Loom-scheduled bounded channel.
pub struct Receiver<T> {
    shared: Arc<Shared<T>>,
}

/// Create a bounded channel holding at most `capacity` messages.
///
/// # Examples
///
/// ```ignore
/// let (tx, rx) = bounded(1);
/// tx.send(1).expect("send");
/// assert!(tx.try_send(2).is_err());
/// assert_eq!(rx.recv(), Ok(1));
/// ```
pub fn bounded<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            queue: VecDeque::new(),
            capacity: capacity.max(1),
            senders: 1,
            receivers: 1,
        }),
        changed: Condvar::new(),
    });
    (
        Sender {
            shared: Arc::clone(&shared),
        },
        Receiver { shared },
    )
}

impl<T> Sender<T> {
    /// Enqueue `message`, waiting while the channel is full.
    ///
    /// # Errors
    ///
    /// Returns the message when every receiver has been dropped.
    pub fn send(&self, message: T) -> Result<(), SendError<T>> {
        let mut state = self.shared.lock();
        loop {
            if state.receivers == 0 {
                return Err(SendError(message));
            }
            if state.queue.len() < state.capacity {
                state.queue.push_back(message);
                self.shared.changed.notify_all();
                return Ok(());
            }
            state = self.shared.wait(state);
        }
    }

    /// Enqueue `message` only if there is room now.
    ///
    /// # Errors
    ///
    /// Returns the message as `Full` or `Disconnected`.
    pub fn try_send(&self, message: T) -> Result<(), TrySendError<T>> {
        let mut state = self.shared.lock();
        if state.receivers == 0 {
            return Err(TrySendError::Disconnected(message));
        }
        if state.queue.len() >= state.capacity {
            return Err(TrySendError::Full(message));
        }
        state.queue.push_back(message);
        self.shared.changed.notify_all();
        Ok(())
    }

    /// Enqueue `message`, waiting without a bound while the channel is full.
    ///
    /// Loom has no clock, so the timeout is ignored; see the seam's module
    /// documentation.
    ///
    /// # Errors
    ///
    /// Returns the message as `Disconnected` when every receiver has been
    /// dropped. It never reports `Timeout`.
    pub fn send_timeout(&self, message: T, _timeout: Duration) -> Result<(), SendTimeoutError<T>> {
        self.send(message)
            .map_err(|SendError(message)| SendTimeoutError::Disconnected(message))
    }
}

impl<T> Receiver<T> {
    /// Dequeue the next message, waiting while the channel is empty.
    ///
    /// # Errors
    ///
    /// Returns `RecvError` once the channel is empty and every sender has been
    /// dropped.
    pub fn recv(&self) -> Result<T, RecvError> {
        let mut state = self.shared.lock();
        loop {
            if let Some(message) = state.queue.pop_front() {
                self.shared.changed.notify_all();
                return Ok(message);
            }
            if state.senders == 0 {
                return Err(RecvError);
            }
            state = self.shared.wait(state);
        }
    }

    /// Dequeue the next message only if one is waiting now.
    ///
    /// # Errors
    ///
    /// Returns `Empty`, or `Disconnected` once the channel is empty and every
    /// sender has been dropped.
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        let mut state = self.shared.lock();
        if let Some(message) = state.queue.pop_front() {
            self.shared.changed.notify_all();
            return Ok(message);
        }
        if state.senders == 0 {
            return Err(TryRecvError::Disconnected);
        }
        Err(TryRecvError::Empty)
    }

    /// Dequeue the next message, waiting without a bound.
    ///
    /// Loom has no clock, so the timeout is ignored; see the seam's module
    /// documentation.
    ///
    /// # Errors
    ///
    /// Returns `Disconnected` once the channel is empty and every sender has
    /// been dropped. It never reports `Timeout`.
    pub fn recv_timeout(&self, _timeout: Duration) -> Result<T, RecvTimeoutError> {
        self.recv()
            .map_err(|RecvError| RecvTimeoutError::Disconnected)
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        self.shared.lock().senders += 1;
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        self.shared.lock().receivers += 1;
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        self.shared.lock().senders -= 1;
        self.shared.changed.notify_all();
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.shared.lock().receivers -= 1;
        self.shared.changed.notify_all();
    }
}

/// Iterates over received messages until the channel disconnects.
pub struct IntoIter<T> {
    receiver: Receiver<T>,
}

impl<T> Iterator for IntoIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        self.receiver.recv().ok()
    }
}

impl<T> IntoIterator for Receiver<T> {
    type Item = T;
    type IntoIter = IntoIter<T>;

    fn into_iter(self) -> IntoIter<T> {
        IntoIter { receiver: self }
    }
}
