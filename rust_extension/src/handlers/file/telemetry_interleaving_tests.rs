//! Controlled interleavings of `handle()`, `flush()` and repeated `close()`.
//!
//! The single-call cases in [`super::telemetry_binding_tests`] and
//! [`super::telemetry_wrapper_failure_tests`] pin one outcome per call. These
//! cases pin what happens when calls meet each other: two closers inside one
//! shared shutdown, a close that times out and is retried, a flush waiting for
//! its acknowledgement when a close takes the shutdown, and calls that arrive
//! while the worker is still parked. Every schedule is driven by the scripted
//! writer's gates rather than by sleeps, so each interleaving is the one the
//! test chose. The same schedules run across the core handler and both rotating
//! wrappers, two queue capacities and all three overflow policies, which states
//! the dimensions the hand-off has to be invariant under.

use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pyo3::{Py, PyAny, Python};
use rstest::rstest;

use super::OverflowPolicy;
use super::scripted_writer::{ScriptedWriter, Stage, expect_stage, handler_with_capacity};
use super::telemetry_binding_tests::{
    assert_handler_error, call_close, call_flush, call_handle, call_handle_error, fill_queue,
    public_count,
};
use super::telemetry_wrapper_binding_tests::{
    rotating_over_writer_with_capacity as rotating,
    timed_rotating_over_writer_with_capacity as timed_rotating,
};

/// A handler builder that takes the case's writer, policy and capacity.
type Builder = fn(Python<'_>, ScriptedWriter, OverflowPolicy, usize) -> Py<PyAny>;

/// Build the core file handler over the case's writer.
///
/// The wrappers above it are the Python-facing surface, but the core handler
/// owns the shutdown hand-off, so the matrix has to reach it directly as well.
fn file_over_writer(
    py: Python<'_>,
    writer: ScriptedWriter,
    policy: OverflowPolicy,
    capacity: usize,
) -> Py<PyAny> {
    Py::new(py, handler_with_capacity(writer, policy, capacity))
        .expect("the handler must be a valid pyclass")
        .into_any()
}

/// One controlled interleaving the matrix runs.
#[derive(Clone, Copy, Debug)]
enum Schedule {
    /// Two closers overlap while the worker is parked inside `write`.
    OverlappingClosers,
    /// A close times out, the worker is released, and a second close completes.
    RetryAfterTimeout,
    /// After a completed close, every later call is refused.
    CallsAfterClose,
    /// A record refused by a full queue is followed by a timed-out close and a
    /// successful retry.
    RefusedHandleThenRetry,
    /// The same schedule, with an enqueue deadline expiring instead of a
    /// refusal under the drop policy.
    TimedOutHandleThenRetry,
    /// A flush already waiting when a close takes the shutdown, plus the calls
    /// that arrive while the worker is still parked.
    WaitingFlush,
}

impl Schedule {
    /// Overflow policy the schedule's queue path depends on.
    fn policy(self) -> OverflowPolicy {
        match self {
            Self::RefusedHandleThenRetry => OverflowPolicy::Drop,
            Self::TimedOutHandleThenRetry => OverflowPolicy::Timeout(Duration::from_millis(20)),
            _ => OverflowPolicy::Block,
        }
    }

    /// Outcome a refused `handle()` must record, when the schedule refuses one.
    const fn refused_outcome(self) -> Option<&'static str> {
        match self {
            Self::RefusedHandleThenRetry => Some("handle_queue_full"),
            Self::TimedOutHandleThenRetry => Some("handle_timed_out"),
            _ => None,
        }
    }

    /// Play the whole schedule out and assert the counters it must leave.
    fn run(self, handler: &Py<PyAny>, writer: &ScriptedWriter, entered: &mpsc::Receiver<Stage>) {
        match self {
            Self::OverlappingClosers => self.overlap(handler, writer, entered),
            Self::RetryAfterTimeout => self.retry_after_timeout(handler, writer),
            Self::CallsAfterClose => self.calls_after_close(handler, writer, entered),
            Self::RefusedHandleThenRetry | Self::TimedOutHandleThenRetry => {
                self.refused_handle_then_retry(handler, writer, entered);
            }
            Self::WaitingFlush => self.waiting_flush(handler, writer, entered),
        }
    }

    /// Hold two closers in the shared shutdown at once, then release both.
    fn overlap(
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
    fn retry_after_timeout(self, handler: &Py<PyAny>, writer: &ScriptedWriter) {
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
    fn calls_after_close(
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
    fn refused_handle_then_retry(
        self,
        handler: &Py<PyAny>,
        writer: &ScriptedWriter,
        entered: &mpsc::Receiver<Stage>,
    ) {
        Python::attach(|py| fill_queue(handler.bind(py), entered));
        assert_handler_error(
            &call_handle_error(handler, "third"),
            "a record the full queue cannot take must raise the handler error",
        );

        call_close(handler);
        assert_eq!(
            count(handler, "close_timed_out"),
            1,
            "the parked worker is stuck"
        );
        if let Some(outcome) = self.refused_outcome() {
            assert_eq!(count(handler, outcome), 1, "the refusal is counted once");
        }

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
    fn waiting_flush(
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
        // closer enters the wait after taking it; the waiting flush cannot
        // leave its own wait until the writer is released.
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
}

/// Read one counter from the handler's public diagnostics map.
fn count(handler: &Py<PyAny>, key: &str) -> u64 {
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

/// Every controlled interleaving counts its real waits and settles the gauge.
///
/// The schedules run across the core handler and both rotating wrappers, over a
/// narrow and a wide queue, and under the block, drop and timeout policies. A
/// handler that lost the shared completion, double-counted a closer, refused a
/// call that was already inside its wait, or counted a refused call as a wait
/// fails here, and the closing gauge assertion catches a wait that never left
/// its scope.
#[rstest]
#[case::rotating_overlap(rotating as Builder, Schedule::OverlappingClosers, 1)]
#[case::rotating_overlap_wide(rotating as Builder, Schedule::OverlappingClosers, 4)]
#[case::rotating_retry(rotating as Builder, Schedule::RetryAfterTimeout, 1)]
#[case::rotating_calls_after_close(rotating as Builder, Schedule::CallsAfterClose, 1)]
#[case::rotating_refused(rotating as Builder, Schedule::RefusedHandleThenRetry, 1)]
#[case::rotating_timed_out(rotating as Builder, Schedule::TimedOutHandleThenRetry, 1)]
#[case::rotating_waiting_flush(rotating as Builder, Schedule::WaitingFlush, 1)]
#[case::rotating_waiting_flush_wide(rotating as Builder, Schedule::WaitingFlush, 4)]
#[case::timed_rotating_overlap(timed_rotating as Builder, Schedule::OverlappingClosers, 1)]
#[case::timed_rotating_overlap_wide(timed_rotating as Builder, Schedule::OverlappingClosers, 4)]
#[case::timed_rotating_retry(timed_rotating as Builder, Schedule::RetryAfterTimeout, 1)]
#[case::timed_rotating_after_close(timed_rotating as Builder, Schedule::CallsAfterClose, 1)]
#[case::timed_rotating_refused(timed_rotating as Builder, Schedule::RefusedHandleThenRetry, 1)]
#[case::timed_rotating_timed_out(timed_rotating as Builder, Schedule::TimedOutHandleThenRetry, 1)]
#[case::timed_rotating_waiting_flush(timed_rotating as Builder, Schedule::WaitingFlush, 1)]
#[case::timed_rotating_waiting_flush_wide(timed_rotating as Builder, Schedule::WaitingFlush, 4)]
#[case::file_overlap(file_over_writer as Builder, Schedule::OverlappingClosers, 1)]
#[case::file_overlap_wide(file_over_writer as Builder, Schedule::OverlappingClosers, 4)]
#[case::file_retry(file_over_writer as Builder, Schedule::RetryAfterTimeout, 1)]
#[case::file_after_close(file_over_writer as Builder, Schedule::CallsAfterClose, 1)]
#[case::file_refused(file_over_writer as Builder, Schedule::RefusedHandleThenRetry, 1)]
#[case::file_timed_out(file_over_writer as Builder, Schedule::TimedOutHandleThenRetry, 1)]
#[case::file_waiting_flush(file_over_writer as Builder, Schedule::WaitingFlush, 1)]
#[case::file_waiting_flush_wide(file_over_writer as Builder, Schedule::WaitingFlush, 4)]
fn each_controlled_interleaving_counts_only_real_waits(
    #[case] make_handler: Builder,
    #[case] schedule: Schedule,
    #[case] capacity: usize,
) {
    let (entered_tx, entered_rx) = mpsc::channel();
    let writer = ScriptedWriter::blocking(entered_tx);
    let handler =
        Python::attach(|py| make_handler(py, writer.clone(), schedule.policy(), capacity));

    schedule.run(&handler, &writer, &entered_rx);

    assert_eq!(
        count(&handler, "waits_in_flight"),
        0,
        "every finished wait must leave the gauge",
    );
}
