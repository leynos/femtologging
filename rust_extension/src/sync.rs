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
//! The timed waits, `recv_timeout` and `send_timeout`, differ in meaning and
//! not only in spelling under Loom: they wait without a bound, because Loom has
//! no clock and a timeout would fire on a schedule Loom chose. A worker that
//! can fail to answer then hangs the model instead of failing it, which the
//! scheduled lane bounds with its job timeout.
//!
//! The channel error types are `crossbeam_channel`'s in both configurations,
//! so callers match on the same variants whichever arm is compiled.

use std::time::Duration;

pub(crate) use crossbeam_channel::{RecvError, RecvTimeoutError, SendTimeoutError, TrySendError};

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

/// Spawn a worker thread with a name, so a stack dump identifies it.
///
/// Named so the dispatch shards are recognisable in a backtrace, a
/// `/proc/<pid>/task/<tid>/comm`, or a debugger — an anonymous pool thread
/// would leave an operator guessing whose handler is stuck.
///
/// # Errors
///
/// Returns the operating system's refusal when a thread cannot be started,
/// which is a resource limit rather than a bug: the caller decides whether the
/// pool can still serve without that shard.
///
/// # Examples
///
/// ```ignore
/// let worker = spawn_named("example-worker".to_string(), || 2 + 2)
///     .expect("a thread should be startable");
/// assert_eq!(worker.join().ok(), Some(4));
/// ```
#[cfg(not(loom))]
pub(crate) fn spawn_named<F, T>(name: String, body: F) -> std::io::Result<JoinHandle<T>>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    std::thread::Builder::new().name(name).spawn(body)
}

/// Spawn a named worker as a Loom model thread.
///
/// Loom has no thread names — a model thread is a coroutine, not a process
/// thread — so the name is dropped and the stack sizing of [`spawn`] applies.
///
/// # Errors
///
/// Loom's builder cannot fail; its `io::Result` mirrors the standard
/// library's signature so callers need no second code path.
///
/// # Examples
///
/// ```ignore
/// let worker = spawn_named("example-worker".to_string(), || 2 + 2)
///     .expect("a Loom thread cannot fail to start");
/// assert_eq!(worker.join().ok(), Some(4));
/// ```
#[cfg(loom)]
pub(crate) fn spawn_named<F, T>(name: String, body: F) -> std::io::Result<JoinHandle<T>>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let _ = name;
    loom::thread::Builder::new()
        .stack_size(LOOM_WORKER_STACK_WORDS)
        .spawn(body)
}

/// Block for at most `timeout` waiting for a message.
///
/// Outside Loom this is `crossbeam_channel`'s timed receive; under Loom the
/// seam's channel ignores the bound, because Loom has no clock and a timeout
/// would fire on a schedule Loom chose. A waiter that can fail to be answered
/// therefore hangs a model instead of failing it, which is why no model relies
/// on a timeout firing.
///
/// # Examples
///
/// ```ignore
/// let (tx, rx) = bounded::<u8>(1);
/// tx.send(7).expect("send");
/// assert_eq!(recv_timeout(&rx, Duration::from_millis(1)), Ok(7));
/// ```
pub(crate) fn recv_timeout<T>(
    receiver: &Receiver<T>,
    timeout: Duration,
) -> Result<T, RecvTimeoutError> {
    receiver.recv_timeout(timeout)
}

#[cfg(test)]
#[path = "sync_tests.rs"]
mod sync_tests;
