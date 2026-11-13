//! Behavioural tests for stream-handler shutdown and flush bounds.
//!
//! The worker formats and writes on its own thread, so `close()` and `flush()`
//! can only be proven bounded by making the worker block on something the test
//! controls. Each blocking destructor or writer below waits for an explicit
//! release signal and reports when it has finished, so the tests assert on
//! observed state rather than on elapsed time.

use std::io;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use _femtologging_rs::{
    DefaultFormatter, FemtoFormatter, FemtoLevel, FemtoLogRecord, FemtoStreamHandler,
};

#[path = "test_utils/captured_log.rs"]
mod captured_log;
#[path = "test_utils/handle_expect.rs"]
mod handle_expect;
#[path = "test_utils/waiter.rs"]
mod waiter;
use captured_log::captured_log;
use handle_expect::HandleExpect;
use waiter::WaiterObservation;

/// Upper bound on how long a test destructor waits for its release signal.
///
/// The bound only stops a broken test from hanging the worker forever; a
/// passing test releases the destructor long before it expires.
const DESTRUCTOR_WAIT: Duration = Duration::from_secs(10);

/// Slack allowed above a configured timeout before a measured wait counts as
/// unbounded.
///
/// A closer that honours its timeout returns within it; one that joins the
/// worker instead waits for the destructor, which is held for at most
/// [`DESTRUCTOR_WAIT`]. Five seconds keeps that ten-second difference intact
/// on a loaded host while tolerating genuine scheduling delay.
const SCHEDULING_MARGIN: Duration = Duration::from_secs(5);

/// Formatter whose destructor blocks until the test releases it.
struct SlowDropFormatter {
    /// Signals that the destructor has been entered and is about to block.
    ///
    /// The test waits for this before claiming that a closer is still blocked,
    /// so a worker that never reached teardown cannot make the claim
    /// vacuously true.
    entered: Sender<()>,
    /// The destructor waits on this channel before returning.
    release: Mutex<Receiver<()>>,
    /// The destructor reports completion on this channel.
    dropped: Sender<()>,
}

impl FemtoFormatter for SlowDropFormatter {
    fn format(&self, record: &FemtoLogRecord) -> String {
        DefaultFormatter.format(record)
    }
}

impl Drop for SlowDropFormatter {
    fn drop(&mut self) {
        // Report arrival before blocking, so the test knows teardown has
        // started rather than merely being slow to begin.
        let _ = self.entered.send(());
        let release = self.release.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = release.recv_timeout(DESTRUCTOR_WAIT);
        let _ = self.dropped.send(());
    }
}

/// A blocked formatter destructor must not extend `close()` past its timeout.
///
/// The destructor reports entry before blocking, so the test knows teardown
/// has started before it claims the closer is still waiting.
#[test]
fn stream_handler_close_is_bounded_by_flush_timeout_when_formatter_drop_blocks() {
    const FLUSH_TIMEOUT: Duration = Duration::from_millis(50);

    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (dropped_tx, dropped_rx) = mpsc::channel();
    let handler = FemtoStreamHandler::with_capacity_timeout(
        io::sink(),
        SlowDropFormatter {
            entered: entered_tx,
            release: Mutex::new(release_rx),
            dropped: dropped_tx,
        },
        8,
        FLUSH_TIMEOUT,
    );
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "hello"));

    let started = Instant::now();
    handler.close();
    let elapsed = started.elapsed();

    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("worker should enter the formatter destructor");
    assert!(
        dropped_rx.try_recv().is_err(),
        "close() returned only after the formatter was dropped"
    );
    // The configured timeout, not the blocked destructor, must bound the wait.
    assert!(
        elapsed >= FLUSH_TIMEOUT,
        "close() returned after {elapsed:?}, before its {FLUSH_TIMEOUT:?} timeout"
    );
    assert!(
        elapsed < FLUSH_TIMEOUT + SCHEDULING_MARGIN,
        "close() waited {elapsed:?} for a destructor that outlived its {FLUSH_TIMEOUT:?} timeout"
    );
    release_tx
        .send(())
        .expect("release the blocked formatter destructor");
    dropped_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("worker should finish formatter teardown");
}

/// A closer must observe completion only after formatter teardown finishes.
///
/// The timeout is long enough for teardown, so the closer cannot return while
/// the destructor is still blocked; the test waits for that state rather than
/// sampling a duration.
#[test]
fn stream_handler_close_waits_for_formatter_teardown_when_timeout_permits() {
    const FLUSH_TIMEOUT: Duration = Duration::from_secs(5);

    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (dropped_tx, dropped_rx) = mpsc::channel();
    let handler = Arc::new(FemtoStreamHandler::with_capacity_timeout(
        io::sink(),
        SlowDropFormatter {
            entered: entered_tx,
            release: Mutex::new(release_rx),
            dropped: dropped_tx,
        },
        8,
        FLUSH_TIMEOUT,
    ));
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "hello"));

    // Take the counter before the closer exists, so its rise can only come
    // from the closer started below.
    let waiters = WaiterObservation::acquire();
    let (closed_tx, closed_rx) = mpsc::channel();
    let closer = {
        let handler = Arc::clone(&handler);
        thread::spawn(move || {
            handler.close();
            let _ = closed_tx.send(());
        })
    };
    // The counter is raised from inside `close()`'s wait, so this proves the
    // closer is waiting rather than merely spawned and possibly descheduled.
    waiters.wait_for_call();
    // Both conditions hold before the blocking claim below is made: the
    // closer is inside its wait, and the worker has entered the destructor
    // that will not return until this test releases it.
    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("worker should enter the formatter destructor");

    // The destructor is still blocked, so a closer that waits for teardown
    // cannot have returned. This is a state assertion, not a timing margin:
    // the destructor only finishes when the test releases it below.
    assert!(
        closed_rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "close() returned before formatter teardown completed"
    );
    release_tx
        .send(())
        .expect("release the blocked formatter destructor");
    closed_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("close() should return once teardown completes");
    dropped_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("worker should finish formatter teardown");
    closer.join().expect("closer thread should not panic");
}

/// Channels shared between the thread-local destructor and its test.
#[derive(Debug)]
struct TlsSync {
    /// The destructor reports entry, before it blocks, on this channel.
    entered: Mutex<Sender<()>>,
    /// The destructor blocks until the test sends on this channel.
    release: Mutex<Receiver<()>>,
    /// The destructor reports completion on this channel.
    dropped: Mutex<Sender<()>>,
}

/// Set once by the thread-local destructor test.
static TLS_SYNC: OnceLock<TlsSync> = OnceLock::new();

thread_local! {
    /// Thread-local initialized on the worker thread. Its destructor runs
    /// after the worker body returns, which is exactly the window in which a
    /// closer must not block on an unbounded join.
    static SLOW_TLS_DTOR: SlowTlsDtor = const { SlowTlsDtor };
}

/// Value whose destructor blocks during thread teardown.
struct SlowTlsDtor;

impl Drop for SlowTlsDtor {
    fn drop(&mut self) {
        let Some(sync) = TLS_SYNC.get() else {
            return;
        };
        let entered = sync.entered.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = entered.send(());
        drop(entered);
        let release = sync.release.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = release.recv_timeout(DESTRUCTOR_WAIT);
        drop(release);
        let dropped = sync.dropped.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = dropped.send(());
    }
}

/// Formatter that initializes the blocking thread-local on the worker thread.
struct TlsDtorFormatter;

impl FemtoFormatter for TlsDtorFormatter {
    fn format(&self, record: &FemtoLogRecord) -> String {
        SLOW_TLS_DTOR.with(|_| ());
        DefaultFormatter.format(record)
    }
}

/// A blocked thread-local destructor must not extend `close()` past its timeout.
///
/// Completion is signalled when the worker body ends, before thread-local
/// destruction, so the closer returns on that signal while the destructor is
/// still blocked and is released only by the test.
#[test]
fn stream_handler_close_is_bounded_by_flush_timeout_when_thread_local_drop_blocks() {
    const FLUSH_TIMEOUT: Duration = Duration::from_millis(50);

    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (dropped_tx, dropped_rx) = mpsc::channel();
    TLS_SYNC
        .set(TlsSync {
            entered: Mutex::new(entered_tx),
            release: Mutex::new(release_rx),
            dropped: Mutex::new(dropped_tx),
        })
        .expect("the thread-local sync is installed once");

    let handler =
        FemtoStreamHandler::with_capacity_timeout(io::sink(), TlsDtorFormatter, 8, FLUSH_TIMEOUT);
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "hello"));

    let started = Instant::now();
    handler.close();
    let elapsed = started.elapsed();

    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("worker thread should enter the thread-local destructor");
    assert!(
        dropped_rx.try_recv().is_err(),
        "close() returned only after the thread-local destructor finished"
    );
    // Completion is signalled when the worker body ends, before thread-local
    // destruction, so close must not spend its configured timeout waiting for
    // the blocked destructor.
    assert!(
        elapsed < FLUSH_TIMEOUT + SCHEDULING_MARGIN,
        "close() waited {elapsed:?}; the thread-local destructor outlived the {FLUSH_TIMEOUT:?} timeout"
    );
    release_tx
        .send(())
        .expect("release the blocked thread-local destructor");
    dropped_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("worker should finish thread-local teardown");
}

/// Formatter that panics on the worker thread and reports that it ran.
struct PanicFormatter {
    /// Signals that the worker entered the formatter before it panicked.
    panicked: Sender<()>,
}

impl FemtoFormatter for PanicFormatter {
    fn format(&self, _record: &FemtoLogRecord) -> String {
        let _ = self.panicked.send(());
        panic!("formatter panics on purpose");
    }
}

/// Text the worker logs when its body panics.
const WORKER_PANIC_WARNING: &str = "worker thread panicked";

/// Waits until the capture log receives a warning containing `needle`.
///
/// The capture logger is process-wide and other tests in this binary log
/// alongside this one, so records are matched by text rather than counted.
/// Waiting for the warning makes the worker's panic an observed event before
/// the closer below is measured.
fn wait_for_warning(logger: &mut logtest::Logger, needle: &str, wait: Duration) -> bool {
    let deadline = Instant::now() + wait;
    loop {
        while let Some(record) = logger.pop() {
            if record.args().to_string().contains(needle) {
                return true;
            }
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

/// A worker panic must still signal completion, so `close()` returns promptly.
///
/// The warning is awaited before the closer is measured, proving the failure
/// path ran instead of the closer racing an empty queue; the configured
/// timeout is then far longer than the allowed wait.
#[test]
fn stream_handler_close_returns_promptly_after_a_worker_panic() {
    const FLUSH_TIMEOUT: Duration = Duration::from_secs(10);

    let mut logger = captured_log();
    let (panicked_tx, panicked_rx) = mpsc::channel();
    let handler = FemtoStreamHandler::with_capacity_timeout(
        io::sink(),
        PanicFormatter {
            panicked: panicked_tx,
        },
        8,
        FLUSH_TIMEOUT,
    );
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "boom"));
    panicked_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("worker should enter the formatter before it panics");
    // The warning is logged inside the worker's panic handler, before it
    // signals completion, so seeing it proves the failure path ran rather
    // than the closer merely racing an empty queue.
    assert!(
        wait_for_warning(&mut logger, WORKER_PANIC_WARNING, Duration::from_secs(5)),
        "the worker's panic must be reported through the log facade"
    );

    let started = Instant::now();
    handler.close();
    let elapsed = started.elapsed();
    // Unwinding signals completion, so the closer must not fall back to its
    // configured timeout. Ten seconds with five seconds of scheduling slack
    // still fails a closer that waited out the timeout.
    assert!(
        elapsed + SCHEDULING_MARGIN < FLUSH_TIMEOUT,
        "close() waited {elapsed:?}; a panicked worker must still signal completion"
    );
}

/// Formatter that panics on the worker thread and has a blocking destructor.
///
/// The panic and the delayed teardown meet in the same worker, which is what
/// distinguishes this case from [`PanicFormatter`] alone: completion must wait
/// for the destructor even though the body never reaches its closing
/// statements.
struct PanicSlowDropFormatter {
    /// Signals that the worker entered the formatter before it panicked.
    panicked: Sender<()>,
    /// Signals that the destructor has been entered during unwinding.
    entered: Sender<()>,
    /// The destructor waits on this channel before returning.
    release: Mutex<Receiver<()>>,
    /// The destructor reports completion on this channel.
    dropped: Sender<()>,
}

impl FemtoFormatter for PanicSlowDropFormatter {
    fn format(&self, _record: &FemtoLogRecord) -> String {
        let _ = self.panicked.send(());
        panic!("formatter panics before its destructor can finish");
    }
}

impl Drop for PanicSlowDropFormatter {
    fn drop(&mut self) {
        // Report arrival before blocking, so the test knows unwinding reached
        // teardown rather than merely being slow to begin.
        let _ = self.entered.send(());
        let release = self.release.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = release.recv_timeout(DESTRUCTOR_WAIT);
        let _ = self.dropped.send(());
    }
}

/// A panicking worker must not signal completion before its teardown finishes.
///
/// Unwinding drops the formatter inside the worker's guarded body, so a closer
/// must remain blocked while that destructor is held. The test waits for the
/// destructor to report entry before making the claim, and the timeout is long
/// enough that only a premature completion signal could return early: a closer
/// that saw one would pass the blocked check.
#[test]
fn stream_handler_close_cannot_observe_completion_before_a_panicking_formatter_drops() {
    const FLUSH_TIMEOUT: Duration = Duration::from_secs(5);

    let (panicked_tx, panicked_rx) = mpsc::channel();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (dropped_tx, dropped_rx) = mpsc::channel();
    let handler = Arc::new(FemtoStreamHandler::with_capacity_timeout(
        io::sink(),
        PanicSlowDropFormatter {
            panicked: panicked_tx,
            entered: entered_tx,
            release: Mutex::new(release_rx),
            dropped: dropped_tx,
        },
        8,
        FLUSH_TIMEOUT,
    ));
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "boom"));
    panicked_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("worker should enter the formatter before it panics");
    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("unwinding should reach the formatter destructor");

    let waiters = WaiterObservation::acquire();
    let (closed_tx, closed_rx) = mpsc::channel();
    let closer = {
        let handler = Arc::clone(&handler);
        thread::spawn(move || {
            handler.close();
            let _ = closed_tx.send(());
        })
    };
    // The counter is raised from inside `close()`'s wait. Waiting for it rules
    // out a closer that was descheduled before calling `close()`, which would
    // make the blocked check below pass without a closer ever waiting.
    waiters.wait_for_call();

    // The destructor is still blocked, so completion cannot have been raised:
    // a closer that returned here observed a signal raised before teardown.
    assert!(
        closed_rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "close() returned before the panicking formatter finished teardown"
    );
    release_tx
        .send(())
        .expect("release the blocked formatter destructor");
    closed_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("close() should return once teardown completes");
    dropped_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("worker should finish formatter teardown");
    closer.join().expect("closer thread should not panic");
}
