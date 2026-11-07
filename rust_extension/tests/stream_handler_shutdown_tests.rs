//! Behavioural tests for stream-handler shutdown and flush bounds.
//!
//! The worker formats and writes on its own thread, so `close()` and `flush()`
//! can only be proven bounded by making the worker block on something the test
//! controls. Each blocking destructor or writer below waits for an explicit
//! release signal and reports when it has finished, so the tests assert on
//! observed state rather than on elapsed time.

use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use _femtologging_rs::{
    DefaultFormatter, FemtoFormatter, FemtoLevel, FemtoLogRecord, FemtoStreamHandler,
};

#[path = "test_utils/handle_expect.rs"]
mod handle_expect;
use handle_expect::HandleExpect;

/// Upper bound on how long a test destructor waits for its release signal.
///
/// The bound only stops a broken test from hanging the worker forever; a
/// passing test releases the destructor long before it expires.
const DESTRUCTOR_WAIT: Duration = Duration::from_secs(10);

/// Formatter whose destructor blocks until the test releases it.
struct SlowDropFormatter {
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
        let release = self.release.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = release.recv_timeout(DESTRUCTOR_WAIT);
        let _ = self.dropped.send(());
    }
}

#[test]
fn stream_handler_close_is_bounded_by_flush_timeout_when_formatter_drop_blocks() {
    const FLUSH_TIMEOUT: Duration = Duration::from_millis(50);

    let (release_tx, release_rx) = mpsc::channel();
    let (dropped_tx, dropped_rx) = mpsc::channel();
    let handler = FemtoStreamHandler::with_capacity_timeout(
        io::sink(),
        SlowDropFormatter {
            release: Mutex::new(release_rx),
            dropped: dropped_tx,
        },
        8,
        FLUSH_TIMEOUT,
    );
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "hello"));

    handler.close();

    assert!(
        dropped_rx.try_recv().is_err(),
        "close() returned only after the formatter was dropped"
    );
    release_tx
        .send(())
        .expect("release the blocked formatter destructor");
    dropped_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("worker should finish formatter teardown");
}

#[test]
fn stream_handler_close_waits_for_formatter_teardown_when_timeout_permits() {
    const FLUSH_TIMEOUT: Duration = Duration::from_secs(5);

    let (release_tx, release_rx) = mpsc::channel();
    let (dropped_tx, dropped_rx) = mpsc::channel();
    let handler = Arc::new(FemtoStreamHandler::with_capacity_timeout(
        io::sink(),
        SlowDropFormatter {
            release: Mutex::new(release_rx),
            dropped: dropped_tx,
        },
        8,
        FLUSH_TIMEOUT,
    ));
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "hello"));

    let (closed_tx, closed_rx) = mpsc::channel();
    let closer = {
        let handler = Arc::clone(&handler);
        thread::spawn(move || {
            handler.close();
            let _ = closed_tx.send(());
        })
    };

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

#[test]
fn stream_handler_close_is_bounded_by_flush_timeout_when_thread_local_drop_blocks() {
    const FLUSH_TIMEOUT: Duration = Duration::from_millis(50);

    let (release_tx, release_rx) = mpsc::channel();
    let (dropped_tx, dropped_rx) = mpsc::channel();
    TLS_SYNC
        .set(TlsSync {
            release: Mutex::new(release_rx),
            dropped: Mutex::new(dropped_tx),
        })
        .expect("the thread-local sync is installed once");

    let handler =
        FemtoStreamHandler::with_capacity_timeout(io::sink(), TlsDtorFormatter, 8, FLUSH_TIMEOUT);
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "hello"));

    handler.close();

    assert!(
        dropped_rx.try_recv().is_err(),
        "close() returned only after the thread-local destructor finished"
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

#[test]
fn stream_handler_close_returns_promptly_after_a_worker_panic() {
    const FLUSH_TIMEOUT: Duration = Duration::from_secs(30);

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

    let started = Instant::now();
    handler.close();
    let elapsed = started.elapsed();
    // Unwinding signals completion, so the closer must not fall back to the
    // timeout. The generous factor only catches a hang; it is not a margin.
    assert!(
        elapsed < FLUSH_TIMEOUT / 4,
        "close() waited {elapsed:?}; a panicked worker must still signal completion"
    );
}

/// Writer whose first flush blocks until the test releases it.
struct BlockingFlushBuf {
    /// Signals that the worker has reached the first flush.
    entered: Sender<()>,
    /// The first flush waits for this channel; later flushes return at once.
    release: Receiver<()>,
    /// Set once the first flush has been released.
    released: bool,
}

impl Write for BlockingFlushBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.released {
            let _ = self.entered.send(());
            let _ = self.release.recv_timeout(DESTRUCTOR_WAIT);
            self.released = true;
        }
        Ok(())
    }
}

#[test]
fn stream_handler_flush_returns_false_when_the_queue_stays_full() {
    const FLUSH_TIMEOUT: Duration = Duration::from_millis(100);

    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let handler = FemtoStreamHandler::with_capacity_timeout(
        BlockingFlushBuf {
            entered: entered_tx,
            release: release_rx,
            released: false,
        },
        DefaultFormatter,
        1,
        FLUSH_TIMEOUT,
    );
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "blocking"));
    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("worker should reach the blocking write");
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "queued"));

    let started = Instant::now();
    assert!(
        !handler.flush(),
        "flush() must fail when the worker never drains the queue"
    );
    let elapsed = started.elapsed();
    // The enqueue wait alone is bounded by one flush timeout. The generous
    // factor only catches a hang; it does not measure scheduling jitter.
    assert!(
        elapsed < FLUSH_TIMEOUT * 4,
        "flush() waited {elapsed:?}; queueing must be bounded by the flush timeout"
    );

    release_tx.send(()).expect("release the blocked writer");
    handler.close();
}
