//! Concurrency primitives for the handler and logger workers.
//!
//! Every worker on the logging path spawns, communicates and locks through
//! this module rather than naming `std::thread`, `crossbeam_channel` or
//! `parking_lot` directly. In an ordinary build the names here are plain
//! re-exports of those crates, so production code compiles to what it did
//! before this module existed. Under `--cfg loom` they resolve to Loom's
//! primitives instead, which lets the models in `tests/heavy/` schedule the
//! workers. Without that, a worker started with `std::thread` touches a
//! Loom-instrumented writer from a thread Loom did not create, and Loom aborts
//! the process.
//!
//! The thread, the channel and the locks move together because each alone is
//! not enough: a Loom thread blocked on a `crossbeam_channel` receive, or on a
//! contended `parking_lot` lock, blocks on a primitive Loom cannot schedule,
//! and the model deadlocks instead of exploring.
//!
//! Two operations differ in meaning, not only in spelling, under Loom:
//!
//! - the timed waits, `recv_timeout` and `send_timeout`, wait without a bound,
//!   because Loom has no clock and a timeout would fire on a schedule Loom
//!   chose. A worker that can fail to answer then hangs the model instead of
//!   failing it, which the scheduled lane bounds with its job timeout;
//! - [`recv_either`] prefers its first receiver when both are ready, where
//!   `crossbeam_channel::select!` chooses at random. Loom has no primitive for
//!   a random choice, so the preference is fixed and stated here.
//!
//! The channel error types are `crossbeam_channel`'s in both configurations,
//! so callers match on the same variants whichever arm is compiled.

// The seam lands before any worker uses it; the milestones that move the
// workers onto it remove this allowance with the last of them.
#![allow(
    dead_code,
    unused_imports,
    reason = "the workers move onto the seam in the milestones that follow"
)]

pub(crate) use crossbeam_channel::{RecvError, TryRecvError, TrySendError};

#[cfg(not(loom))]
pub(crate) use crossbeam_channel::{Receiver, Sender, bounded};
#[cfg(not(loom))]
pub(crate) use parking_lot::{Mutex, RwLock};
#[cfg(not(loom))]
pub(crate) use std::thread::{JoinHandle, spawn};

#[cfg(loom)]
mod loom_channel;
#[cfg(loom)]
mod loom_lock;

#[cfg(loom)]
pub(crate) use loom::thread::JoinHandle;
#[cfg(loom)]
pub(crate) use loom_channel::{Receiver, Sender, bounded};
#[cfg(loom)]
pub(crate) use loom_lock::{Mutex, RwLock};

/// Stack size, in machine words, for a worker spawned inside a Loom model.
///
/// Loom runs each model thread as a coroutine on a small fixed stack, and an
/// unoptimized worker formatting a record overflows the default one.
#[cfg(loom)]
const LOOM_WORKER_STACK_WORDS: usize = 1 << 17;

/// Spawn a worker as a Loom model thread with a stack large enough for it.
///
/// Loom's `Builder::spawn` cannot fail; its `io::Result` mirrors the standard
/// library's signature.
///
/// # Examples
///
/// ```ignore
/// let worker = spawn(|| 2 + 2);
/// assert_eq!(worker.join().ok(), Some(4));
/// ```
#[cfg(loom)]
pub(crate) fn spawn<F, T>(body: F) -> JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    match loom::thread::Builder::new()
        .stack_size(LOOM_WORKER_STACK_WORDS)
        .spawn(body)
    {
        Ok(handle) => handle,
        Err(error) => unreachable!("Loom's thread builder cannot fail: {error}"),
    }
}

/// The receiver [`recv_either`] took a message, or a disconnection, from.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Either<A, B> {
    /// The first receiver was ready.
    First(A),
    /// The second receiver was ready.
    Second(B),
}

/// Block until either receiver has a message or is disconnected.
///
/// A disconnected receiver counts as ready, as it does for
/// `crossbeam_channel::select!`, so a worker waiting here observes its
/// shutdown channel being dropped.
///
/// # Examples
///
/// ```ignore
/// let (first_tx, first_rx) = bounded::<u8>(1);
/// let (_second_tx, second_rx) = bounded::<u8>(1);
/// first_tx.send(7).expect("send");
/// assert_eq!(recv_either(&first_rx, &second_rx), Either::First(Ok(7)));
/// ```
#[cfg(not(loom))]
pub(crate) fn recv_either<A, B>(
    first: &Receiver<A>,
    second: &Receiver<B>,
) -> Either<Result<A, RecvError>, Result<B, RecvError>> {
    crossbeam_channel::select! {
        recv(first) -> message => Either::First(message),
        recv(second) -> message => Either::Second(message),
    }
}

/// Block until either receiver has a message or is disconnected.
///
/// Loom has no multi-channel wait, so this polls both receivers and yields to
/// the Loom scheduler between rounds. A yielded thread is not rescheduled
/// while another thread can run, so the poll costs no extra exploration unless
/// every other thread is blocked, in which case the loop is a genuine hang
/// that Loom reports as one.
///
/// # Examples
///
/// ```ignore
/// let (first_tx, first_rx) = bounded::<u8>(1);
/// let (_second_tx, second_rx) = bounded::<u8>(1);
/// first_tx.send(7).expect("send");
/// assert_eq!(recv_either(&first_rx, &second_rx), Either::First(Ok(7)));
/// ```
#[cfg(loom)]
pub(crate) fn recv_either<A, B>(
    first: &Receiver<A>,
    second: &Receiver<B>,
) -> Either<Result<A, RecvError>, Result<B, RecvError>> {
    loop {
        if let Some(outcome) = ready(first.try_recv()) {
            return Either::First(outcome);
        }
        if let Some(outcome) = ready(second.try_recv()) {
            return Either::Second(outcome);
        }
        loom::thread::yield_now();
    }
}

/// Map a non-blocking receive onto the outcome a blocking one would report.
///
/// Returns `None` while the channel is merely empty.
#[cfg(loom)]
fn ready<T>(outcome: Result<T, TryRecvError>) -> Option<Result<T, RecvError>> {
    match outcome {
        Ok(message) => Some(Ok(message)),
        Err(TryRecvError::Disconnected) => Some(Err(RecvError)),
        Err(TryRecvError::Empty) => None,
    }
}

#[cfg(test)]
#[path = "sync_tests.rs"]
mod sync_tests;
