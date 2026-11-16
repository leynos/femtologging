//! Shutdown behaviour when the worker panics.
//!
//! Both scenarios make the worker unwind after a formatter has already
//! signalled the test, so a closer that returns early can only have observed
//! a completion signal raised before teardown finished.

use std::io;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use _femtologging_rs::{FemtoFormatter, FemtoLevel, FemtoLogRecord, FemtoStreamHandler};

use super::captured_log::captured_log;
use super::handle_expect::HandleExpect;
use super::waiter::{WaiterObservation, serialise};
use super::{DESTRUCTOR_WAIT, SCHEDULING_MARGIN};

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
    // This test's close moves the process-wide waiter counter, so it must not
    // run beside a test that observes it.
    let _serial = serialise();

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
    // Take the counter before the handler exists: setup failure, an unwind and
    // teardown all stay inside the guard, so its rise can only come from the
    // closer started below.
    let waiters = WaiterObservation::acquire();

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
