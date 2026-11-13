//! Bodies of the controlled `handle()`, `flush()` and `close()` interleavings.
//!
//! Each schedule parks the worker at a point the scripted writer reports and
//! then meets that state with the calls under test: two closers inside one
//! shared shutdown, a close that times out and is retried, a record already
//! waiting for capacity when a close takes the shutdown, a flush waiting for
//! its acknowledgement at the same moment, and calls that arrive while the
//! worker is still parked. Every step is gated by the scripted writer or by the
//! public in-flight gauge, so no interleaving depends on a sleep. The matrix
//! that runs these schedules across builders, capacities and policies lives in
//! [`super::telemetry_interleaving_tests`].

use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pyo3::{Py, PyAny, Python};

use super::scripted_writer::{ScriptedWriter, Stage, expect_stage};
use super::telemetry_binding_tests::{
    assert_handler_error, call_close, call_flush, call_handle, call_handle_error, fill_queue,
    public_count,
};
use super::telemetry_interleaving_tests::Schedule;

impl Schedule {
    /// Outcome a refused `handle()` must record.
    pub(super) const fn refused_outcome(self) -> &'static str {
        match self {
            Self::TimedOutHandleThenRetry => "handle_timed_out",
            _ => "handle_queue_full",
        }
    }

    /// Hold two closers in the shared shutdown at once, then release both.
    pub(super) fn overlap(
        self,
        handler: &Py<PyAny>,
        writer: &ScriptedWriter,
        entered: &mpsc::Receiver<Stage>,
    ) {
        call_handle(handler, "park");
        expect_stage(entered, Stage::Write);

        let closers: Vec<_> = (0..2).map(|_| spawn_close(handler)).collect();
        // Both closers release the interpreter lock before waiting, so polling
        // from this thread cannot hold either of them up.
        assert_eq!(
            await_gauge(handler, 2),
            2,
            "both closers must share the shutdown",
        );

        writer.release();
        join_closers(closers);

        assert_eq!(count(handler, "close_completed"), 2, "both must complete");
        assert_eq!(count(handler, "close_timed_out"), 0, "neither may time out");
    }

    /// Time a close out, release the worker, then observe the retry.
    pub(super) fn retry_after_timeout(self, handler: &Py<PyAny>, writer: &ScriptedWriter) {
        call_close(handler);
        assert_eq!(
            count(handler, "close_timed_out"),
            1,
            "the close must time out"
        );
        assert_eq!(
            count(handler, "close_completed"),
            0,
            "a timeout is not a completion"
        );

        writer.release();
        call_close(handler);
        assert_eq!(
            count(handler, "close_completed"),
            1,
            "the retry must complete"
        );
        assert_eq!(
            count(handler, "close_timed_out"),
            1,
            "the timeout stays counted"
        );
    }

    /// Close cleanly, then check every later call is refused.
    pub(super) fn calls_after_close(
        self,
        handler: &Py<PyAny>,
        writer: &ScriptedWriter,
        entered: &mpsc::Receiver<Stage>,
    ) {
        // No record is queued, so the worker's first stage is the terminal
        // flush; releasing the gate there lets the first close complete.
        let closer = spawn_close(handler);
        expect_stage(entered, Stage::Flush);
        writer.release();
        closer.join().expect("the closer thread must not panic");
        assert_eq!(
            count(handler, "close_completed"),
            1,
            "the close must complete"
        );

        assert_handler_error(
            &call_handle_error(handler, "late"),
            "a call after close must raise the handler error",
        );
        assert!(!call_flush(handler), "a late flush cannot be acknowledged");
        call_close(handler);

        assert_eq!(
            count(handler, "handle_closed"),
            1,
            "the record counts as closed"
        );
        assert_eq!(
            count(handler, "flush_closed"),
            1,
            "the flush counts as closed"
        );
        assert_eq!(
            count(handler, "close_completed"),
            2,
            "a repeat close completes"
        );
    }

    /// Refuse one record, time the close out, then finish it on a retry.
    pub(super) fn refused_handle_then_retry(
        self,
        handler: &Py<PyAny>,
        writer: &ScriptedWriter,
        entered: &mpsc::Receiver<Stage>,
        capacity: usize,
    ) {
        Python::attach(|py| fill_queue(handler.bind(py), entered, capacity));
        assert_handler_error(
            &call_handle_error(handler, "refused"),
            "a record the full queue cannot take must raise the handler error",
        );

        call_close(handler);
        assert_eq!(
            count(handler, "close_timed_out"),
            1,
            "the parked worker is stuck"
        );
        assert_eq!(
            count(handler, self.refused_outcome()),
            1,
            "the refusal is counted once"
        );

        writer.release();
        call_close(handler);
        assert_eq!(
            count(handler, "close_completed"),
            1,
            "the retry must complete"
        );
        assert_eq!(
            count(handler, "close_timed_out"),
            1,
            "the timeout stays counted"
        );
    }

    /// Answer a flush that was already waiting when a close took the shutdown.
    ///
    /// The worker is parked inside the writer's flush with the acknowledgement
    /// still outstanding, so the flush is inside its wait when the close takes
    /// the shutdown. Calls from this thread then meet the closed handler while
    /// the worker is still parked. Releasing the writer answers the waiting
    /// flush first, so it must record the acknowledgement rather than a
    /// closed-handler outcome.
    pub(super) fn waiting_flush(
        self,
        handler: &Py<PyAny>,
        writer: &ScriptedWriter,
        entered: &mpsc::Receiver<Stage>,
    ) {
        let flusher = spawn_flush(handler);
        expect_stage(entered, Stage::Flush);
        assert_eq!(await_gauge(handler, 1), 1, "the flush must be waiting");

        let closer = spawn_close(handler);
        // A reading of two is taken only after `take_shutdown`, because the
        // closer enters its wait after taking the sender; the waiting flush
        // cannot leave its own wait until the writer is released. The gauge
        // marks the close's entry into its wait, and the closed-handler
        // outcomes below independently prove the sender was already gone.
        assert_eq!(await_gauge(handler, 2), 2, "the close must join the flush");

        assert_handler_error(
            &call_handle_error(handler, "late"),
            "a call during shutdown must raise the handler error",
        );
        assert!(!call_flush(handler), "a flush during shutdown is refused");

        writer.release();
        flusher.join().expect("the flush thread must not panic");
        closer.join().expect("the closer thread must not panic");

        assert_eq!(
            count(handler, "flush_acknowledged"),
            1,
            "the flush is answered"
        );
        assert_eq!(
            count(handler, "handle_closed"),
            1,
            "the call counts as closed"
        );
        assert_eq!(
            count(handler, "flush_closed"),
            1,
            "the flush counts as closed"
        );
        assert_eq!(
            count(handler, "close_completed"),
            1,
            "the close must complete"
        );
        assert_eq!(count(handler, "close_timed_out"), 0, "nothing may time out");
    }

    /// Accept a record that was waiting for capacity when close took over.
    ///
    /// The queue is full and the worker parked, so the spawned `handle()` is
    /// inside its wait when `close()` takes the sender. Releasing the writer
    /// frees capacity, so the blocked record must still be accepted rather than
    /// reported as a closed call, and the close must then complete.
    pub(super) fn blocked_handle_then_shutdown(
        self,
        handler: &Py<PyAny>,
        writer: &ScriptedWriter,
        entered: &mpsc::Receiver<Stage>,
        capacity: usize,
    ) {
        Python::attach(|py| fill_queue(handler.bind(py), entered, capacity));

        let blocked = spawn_handle(handler, "blocked");
        assert_eq!(await_gauge(handler, 1), 1, "the blocked call must wait");

        let closer = spawn_close(handler);
        assert_eq!(
            await_gauge(handler, 2),
            2,
            "the close must join the blocked call",
        );

        writer.release();
        blocked.join().expect("the blocked call must not panic");
        closer.join().expect("the closer must not panic");

        // The fill accepted one record per slot plus the one the worker took;
        // the blocked call is the only other acceptance.
        assert_eq!(
            count(handler, "handle_accepted"),
            capacity as u64 + 2,
            "the blocked record is accepted once capacity returns",
        );
        assert_eq!(
            count(handler, "handle_closed"),
            0,
            "a blocked record is not a closed call"
        );
        assert_eq!(
            count(handler, "close_completed"),
            1,
            "the close must complete"
        );
        assert_eq!(count(handler, "close_timed_out"), 0, "nothing may time out");
    }
}

/// Read one counter from the handler's public diagnostics map.
pub(super) fn count(handler: &Py<PyAny>, key: &str) -> u64 {
    Python::attach(|py| public_count(handler.bind(py), key))
}

/// Close the handler on another thread so this one can observe the wait.
fn spawn_close(handler: &Py<PyAny>) -> JoinHandle<()> {
    spawn_call(handler, "close")
}

/// Flush the handler on another thread so this one can observe the wait.
fn spawn_flush(handler: &Py<PyAny>) -> JoinHandle<()> {
    spawn_call(handler, "flush")
}

/// Call a no-argument binding on another thread.
fn spawn_call(handler: &Py<PyAny>, method: &'static str) -> JoinHandle<()> {
    let handler = Python::attach(|py| handler.clone_ref(py));
    std::thread::spawn(move || {
        Python::attach(|py| {
            handler
                .call_method0(py, method)
                .expect("a concurrent call must not raise");
        });
    })
}

/// Enqueue one record on another thread so this one can observe the wait.
///
/// The record is only accepted once the released worker frees a slot, so the
/// send has to succeed after the close has already taken the sender.
fn spawn_handle(handler: &Py<PyAny>, message: &'static str) -> JoinHandle<()> {
    let handler = Python::attach(|py| handler.clone_ref(py));
    std::thread::spawn(move || {
        Python::attach(|py| {
            handler
                .call_method1(py, "handle", ("core", "INFO", message))
                .expect("a blocked call that gains capacity must be accepted");
        });
    })
}

/// Join every closer, failing if one panicked.
fn join_closers(closers: Vec<JoinHandle<()>>) {
    for closer in closers {
        closer.join().expect("a closer thread must not panic");
    }
}

/// Poll the public in-flight gauge until it reaches `expected`.
///
/// The last reading is returned so the caller asserts on what it actually saw
/// rather than on the fact that the deadline passed.
fn await_gauge(handler: &Py<PyAny>, expected: u64) -> u64 {
    let deadline = Instant::now() + Duration::from_millis(800);
    loop {
        let gauge = count(handler, "waits_in_flight");
        if gauge == expected || Instant::now() >= deadline {
            return gauge;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}
