//! Matrix of controlled `handle()`, `flush()` and repeated `close()` cases.
//!
//! The single-call cases in [`super::telemetry_binding_tests`] and
//! [`super::telemetry_wrapper_failure_tests`] pin one outcome per call. These
//! cases pin what happens when calls meet each other: two closers inside one
//! shared shutdown, a close that times out and is retried, a record already
//! waiting for capacity when a close takes the shutdown, a flush waiting for
//! its acknowledgement at the same moment, and calls that arrive while the
//! worker is still parked.
//!
//! Every schedule is driven by the scripted writer's gates rather than by
//! sleeps, so each interleaving is the one the test chose. The schedule bodies
//! live in [`super::telemetry_interleaving_schedules`]; this module supplies
//! the builders, capacities and policies they are run under, so the matrix
//! states the dimensions the shutdown hand-off must be invariant under.

use std::panic::AssertUnwindSafe;
use std::sync::mpsc;
use std::time::Duration;

use pyo3::{Py, PyAny, Python};
use rstest::rstest;

use super::OverflowPolicy;
use super::scripted_writer::{ScriptedWriter, Stage, handler_with_capacity};
use super::telemetry_interleaving_schedules::{count, sends};
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

/// One handler builder and the name a failure message reports.
#[derive(Clone, Copy)]
struct BuilderCase {
    /// Name this builder carries into a failure message.
    name: &'static str,
    /// Builds the handler over the case's writer.
    make: Builder,
}

/// The builders every schedule runs across.
const BUILDERS: [BuilderCase; 3] = [
    BuilderCase {
        name: "rotating",
        make: rotating,
    },
    BuilderCase {
        name: "timed_rotating",
        make: timed_rotating,
    },
    BuilderCase {
        name: "file",
        make: file_over_writer,
    },
];

/// Queue capacities every schedule runs at: one slot and a wide queue.
const CAPACITIES: [usize; 2] = [1, 4];

/// The policy a schedule needs, when the policy is part of what it tests.
const BLOCK_ONLY: [OverflowPolicy; 1] = [OverflowPolicy::Block];
const DROP_ONLY: [OverflowPolicy; 1] = [OverflowPolicy::Drop];
const TIMEOUT_ONLY: [OverflowPolicy; 1] = [OverflowPolicy::Timeout(Duration::from_millis(20))];
/// Every policy, for a schedule whose outcome cannot depend on one.
const ALL_POLICIES: [OverflowPolicy; 3] = [
    OverflowPolicy::Block,
    OverflowPolicy::Drop,
    OverflowPolicy::Timeout(Duration::from_millis(20)),
];

/// One built matrix entry: the handler, its worker and the queue width.
///
/// The schedule bodies receive this rather than four loose parameters, so a
/// schedule cannot accidentally mix one case's writer with another's handler.
pub(super) struct Case<'a> {
    /// The handler under test.
    pub(super) handler: &'a Py<PyAny>,
    /// The worker the handler sends its records to.
    pub(super) writer: &'a ScriptedWriter,
    /// The stages the worker reports as it processes commands.
    pub(super) entered: &'a mpsc::Receiver<Stage>,
    /// Queue capacity this case was built with.
    pub(super) capacity: usize,
}

/// One controlled interleaving the matrix runs.
#[derive(Clone, Copy, Debug)]
pub(super) enum Schedule {
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
    /// A record already waiting for capacity when a close takes the shutdown.
    BlockedHandleThenShutdown,
}

impl Schedule {
    /// The name a failure message reports.
    const fn name(self) -> &'static str {
        match self {
            Self::OverlappingClosers => "overlapping_closers",
            Self::RetryAfterTimeout => "retry_after_timeout",
            Self::CallsAfterClose => "calls_after_close",
            Self::RefusedHandleThenRetry => "refused_handle_then_retry",
            Self::TimedOutHandleThenRetry => "timed_out_handle_then_retry",
            Self::WaitingFlush => "waiting_flush",
            Self::BlockedHandleThenShutdown => "blocked_handle_then_shutdown",
        }
    }

    /// The policies this schedule can express.
    ///
    /// A refusal schedule is defined by a policy that refuses, and a blocked
    /// call needs a policy that waits, so those run only under their own
    /// policy. Every other schedule's outcome must not depend on the policy, so
    /// it runs under all three.
    fn policies(self) -> &'static [OverflowPolicy] {
        match self {
            Self::RefusedHandleThenRetry => &DROP_ONLY,
            Self::TimedOutHandleThenRetry => &TIMEOUT_ONLY,
            Self::BlockedHandleThenShutdown => &BLOCK_ONLY,
            _ => &ALL_POLICIES,
        }
    }

    /// Play the whole schedule out and assert the counters it must leave.
    fn run(self, case: &Case<'_>) {
        match self {
            Self::OverlappingClosers => self.overlap(case),
            Self::RetryAfterTimeout => self.retry_after_timeout(case),
            Self::CallsAfterClose => self.calls_after_close(case),
            Self::RefusedHandleThenRetry | Self::TimedOutHandleThenRetry => {
                self.refused_handle_then_retry(case);
            }
            Self::WaitingFlush => self.waiting_flush(case),
            Self::BlockedHandleThenShutdown => self.blocked_handle_then_shutdown(case),
        }
    }
}

/// Every controlled interleaving counts its real waits and settles the gauge.
///
/// Each schedule runs across the core handler and both rotating wrappers, over
/// a one-slot and a four-slot queue, and under every policy that can express
/// it. A handler that lost the shared completion, double-counted a closer,
/// refused a call that was already inside its wait, or counted a refused call
/// as a wait fails here, and the closing gauge assertion catches a wait that
/// never left its scope.
#[rstest]
#[case::overlapping_closers(Schedule::OverlappingClosers)]
#[case::retry_after_timeout(Schedule::RetryAfterTimeout)]
#[case::calls_after_close(Schedule::CallsAfterClose)]
#[case::refused_handle_then_retry(Schedule::RefusedHandleThenRetry)]
#[case::timed_out_handle_then_retry(Schedule::TimedOutHandleThenRetry)]
#[case::waiting_flush(Schedule::WaitingFlush)]
#[case::blocked_handle_then_shutdown(Schedule::BlockedHandleThenShutdown)]
fn each_controlled_interleaving_counts_only_real_waits(#[case] schedule: Schedule) {
    for builder in BUILDERS {
        for &policy in schedule.policies() {
            for capacity in CAPACITIES {
                run_case(builder, schedule, policy, capacity);
            }
        }
    }
}

/// Run one matrix entry, naming the combination if it fails.
fn run_case(builder: BuilderCase, schedule: Schedule, policy: OverflowPolicy, capacity: usize) {
    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let (entered_tx, entered_rx) = mpsc::channel();
        let writer = ScriptedWriter::blocking(entered_tx);
        let handler = Python::attach(|py| (builder.make)(py, writer.clone(), policy, capacity));
        let case = Case {
            handler: &handler,
            writer: &writer,
            entered: &entered_rx,
            capacity,
        };
        schedule.run(&case);
        assert_eq!(
            count(&handler, "waits_in_flight"),
            0,
            "every finished wait must leave the gauge",
        );
        assert_eq!(
            sends(&handler),
            0,
            "every finished send must leave the send gauge",
        );
    }));
    if let Err(payload) = outcome {
        let reason = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or("the case panicked without a message");
        panic!(
            "{}/{}/{capacity} under {}: {reason}",
            builder.name,
            schedule.name(),
            policy_name(policy),
        );
    }
}

/// Name the overflow policy a failure message reports.
const fn policy_name(policy: OverflowPolicy) -> &'static str {
    match policy {
        OverflowPolicy::Block => "block",
        OverflowPolicy::Drop => "drop",
        OverflowPolicy::Timeout(_) => "timeout",
    }
}
