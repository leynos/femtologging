//! Controlled interleavings of `handle()`, `flush()` and repeated `close()`.
//!
//! The single-call cases in [`super::telemetry_binding_tests`] and
//! [`super::telemetry_wrapper_failure_tests`] pin one outcome per call. These
//! cases pin what happens when calls meet each other: two closers inside one
//! shared shutdown, a close that times out and is retried, and calls that
//! arrive after the worker has stopped. Every schedule is driven by the
//! scripted writer's gates rather than by sleeps, so each interleaving is the
//! one the test chose. The same schedules run across both rotating wrappers,
//! two queue capacities and all three overflow policies, which states the
//! dimensions the hand-off has to be invariant under.

use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pyo3::{Py, PyAny, Python};
use rstest::rstest;

use super::OverflowPolicy;
use super::scripted_writer::{ScriptedWriter, Stage, expect_stage};
use super::telemetry_binding_tests::{fill_queue, public_count};
use super::telemetry_wrapper_binding_tests::{
    rotating_over_writer_with_capacity as rotating,
    timed_rotating_over_writer_with_capacity as timed_rotating,
};

/// A wrapper builder that takes the case's writer, policy and capacity.
type Builder = fn(Python<'_>, ScriptedWriter, OverflowPolicy, usize) -> Py<PyAny>;

/// One controlled interleaving the matrix runs.
#[derive(Clone, Copy, Debug)]
enum Schedule {
    /// Two closers overlap while the worker is parked inside `write`.
    OverlappingClosers,
    /// The same overlap over a queue wide enough to stay empty behind the
    /// parked worker, so the hand-off cannot depend on a full queue.
    OverlappingClosersWideQueue,
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
}

impl Schedule {
    /// Queue slots the schedule needs to reach its paths.
    const fn capacity(self) -> usize {
        match self {
            Self::OverlappingClosersWideQueue => 4,
            _ => 1,
        }
    }

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
            Self::OverlappingClosers | Self::OverlappingClosersWideQueue => {
                self.overlap(handler, writer, entered);
            }
            Self::RetryAfterTimeout => self.retry_after_timeout(handler, writer),
            Self::CallsAfterClose => self.calls_after_close(handler, writer, entered),
            Self::RefusedHandleThenRetry | Self::TimedOutHandleThenRetry => {
                self.refused_handle_then_retry(handler, writer, entered);
            }
        }
    }

    /// Hold two closers in the shared shutdown at once, then release both.
    fn overlap(
        self,
        handler: &Py<PyAny>,
        writer: &ScriptedWriter,
        entered: &mpsc::Receiver<Stage>,
    ) {
        handle(handler, "park");
        expect_stage(entered, Stage::Write);

        let closers: Vec<_> = (0..2).map(|_| spawn_close(handler)).collect();
        // Both closers release the interpreter lock before waiting, so polling
        // from this thread cannot hold either of them up.
        assert_eq!(
            await_gauge(handler, 2),
            2,
            "both closers must be counted while they share the shutdown",
        );

        writer.release();
        join_closers(closers);

        assert_eq!(
            count(handler, "close_completed"),
            2,
            "both closers must record the completion the release produced",
        );
        assert_eq!(
            count(handler, "close_timed_out"),
            0,
            "neither closer may time out once the worker is released",
        );
    }

    /// Time a close out, release the worker, then observe the retry.
    fn retry_after_timeout(self, handler: &Py<PyAny>, writer: &ScriptedWriter) {
        close(handler);
        assert_eq!(
            count(handler, "close_timed_out"),
            1,
            "a worker parked in its terminal flush must record a timeout",
        );
        assert_eq!(
            count(handler, "close_completed"),
            0,
            "the timeout must not count as a completion",
        );

        writer.release();
        close(handler);
        assert_eq!(
            count(handler, "close_completed"),
            1,
            "the retry must record the completion it observed",
        );
        assert_eq!(
            count(handler, "close_timed_out"),
            1,
            "the earlier timeout stays counted after the retry succeeds",
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
            "the released worker must let the closer complete",
        );

        assert_handler_error(
            &handle_error(handler, "late"),
            "a call after close must raise the handler error",
        );
        assert!(
            !flush(handler),
            "a flush after close cannot be acknowledged",
        );
        close(handler);

        assert_eq!(
            count(handler, "handle_closed"),
            1,
            "the refused record must be counted against the closed handler",
        );
        assert_eq!(
            count(handler, "flush_closed"),
            1,
            "the refused flush must be counted against the closed handler",
        );
        assert_eq!(
            count(handler, "close_completed"),
            2,
            "a repeated close records the completion it observes",
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
            &handle_error(handler, "third"),
            "a record the full queue cannot take must raise the handler error",
        );

        close(handler);
        assert_eq!(
            count(handler, "close_timed_out"),
            1,
            "the parked worker cannot complete the shutdown",
        );
        if let Some(outcome) = self.refused_outcome() {
            assert_eq!(
                count(handler, outcome),
                1,
                "the refused record must be counted as {outcome}",
            );
        }

        writer.release();
        close(handler);
        assert_eq!(
            count(handler, "close_completed"),
            1,
            "the retry must record the completion the release produced",
        );
        assert_eq!(
            count(handler, "close_timed_out"),
            1,
            "the earlier timeout stays counted after the retry succeeds",
        );
    }
}

/// Call the wrapper's `handle()` and require the record to be accepted.
fn handle(handler: &Py<PyAny>, message: &str) {
    Python::attach(|py| {
        handler
            .call_method1(py, "handle", ("core", "INFO", message))
            .expect("the record must be accepted");
    });
}

/// Call the wrapper's `handle()` and return the error it raised.
fn handle_error(handler: &Py<PyAny>, message: &str) -> pyo3::PyErr {
    Python::attach(|py| {
        handler
            .call_method1(py, "handle", ("core", "INFO", message))
            .expect_err("a call the handler cannot take must raise")
    })
}

/// Assert a refused call surfaces the wrapper's public handler error.
///
/// The two wrappers map a handler failure to different Python classes —
/// `RuntimeError` for rotating and `ValueError` for timed-rotating — so the
/// matrix pins the message both produce instead of a class neither shares.
fn assert_handler_error(error: &pyo3::PyErr, context: &str) {
    Python::attach(|py| {
        let message = error.value(py).to_string();
        assert!(
            message.starts_with("Handler error: "),
            "{context}: {message}",
        );
    });
}

/// Call the wrapper's `flush()` and return its acknowledgement.
fn flush(handler: &Py<PyAny>) -> bool {
    Python::attach(|py| {
        handler
            .call_method0(py, "flush")
            .expect("flush must not raise")
            .extract(py)
            .expect("flush must return a bool")
    })
}

/// Call the wrapper's `close()`, which must not raise even on a timeout.
fn close(handler: &Py<PyAny>) {
    Python::attach(|py| {
        handler
            .call_method0(py, "close")
            .expect("close must not raise");
    });
}

/// Read one counter from the wrapper's public diagnostics map.
fn count(handler: &Py<PyAny>, key: &str) -> u64 {
    Python::attach(|py| public_count(handler.bind(py), key))
}

/// Close the wrapper on another thread so this one can observe the wait.
fn spawn_close(handler: &Py<PyAny>) -> JoinHandle<()> {
    let handler = Python::attach(|py| handler.clone_ref(py));
    std::thread::spawn(move || {
        Python::attach(|py| {
            handler
                .call_method0(py, "close")
                .expect("a concurrent close must not raise");
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
/// The schedules are run across both rotating wrappers, over a narrow and a
/// wide queue, and under the block, drop and timeout policies. A wrapper that
/// lost the shared completion, double-counted a closer, or counted a refused
/// call as a wait fails here, and the closing gauge assertion catches a wait
/// that never left its scope.
#[rstest]
#[case::rotating_overlap(rotating as Builder, Schedule::OverlappingClosers)]
#[case::rotating_overlap_wide_queue(rotating as Builder, Schedule::OverlappingClosersWideQueue)]
#[case::rotating_retry(rotating as Builder, Schedule::RetryAfterTimeout)]
#[case::rotating_calls_after_close(rotating as Builder, Schedule::CallsAfterClose)]
#[case::rotating_refused_handle(rotating as Builder, Schedule::RefusedHandleThenRetry)]
#[case::rotating_timed_out_handle(rotating as Builder, Schedule::TimedOutHandleThenRetry)]
#[case::timed_rotating_overlap(timed_rotating as Builder, Schedule::OverlappingClosers)]
#[case::timed_rotating_overlap_wide_queue(timed_rotating as Builder, Schedule::OverlappingClosersWideQueue)]
#[case::timed_rotating_retry(timed_rotating as Builder, Schedule::RetryAfterTimeout)]
#[case::timed_rotating_calls_after_close(timed_rotating as Builder, Schedule::CallsAfterClose)]
#[case::timed_rotating_refused_handle(timed_rotating as Builder, Schedule::RefusedHandleThenRetry)]
#[case::timed_rotating_timed_out_handle(timed_rotating as Builder, Schedule::TimedOutHandleThenRetry)]
fn each_controlled_interleaving_counts_only_real_waits(
    #[case] make_wrapper: Builder,
    #[case] schedule: Schedule,
) {
    let (entered_tx, entered_rx) = mpsc::channel();
    let writer = ScriptedWriter::blocking(entered_tx);
    let handler = Python::attach(|py| {
        make_wrapper(py, writer.clone(), schedule.policy(), schedule.capacity())
    });

    schedule.run(&handler, &writer, &entered_rx);

    assert_eq!(
        count(&handler, "waits_in_flight"),
        0,
        "every finished wait must leave the gauge",
    );
}
