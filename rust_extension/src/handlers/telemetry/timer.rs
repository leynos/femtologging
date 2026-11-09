//! Timing one wait and emitting its bounded internal event.
//!
//! The timer starts before the detached section and finishes after it returns,
//! on the interpreter-owning thread. That placement makes the recorded interval
//! the whole wait rather than only the enqueue or the acknowledgement, and
//! ensures a tracing subscriber never has to attach to Python from a thread
//! that holds no GIL.

use std::time::{Duration, Instant};

use super::diagnostics::WaitDiagnostics;
use super::outcome::WaitOutcome;

/// Names the Python entry point and handler class a wait belongs to.
///
/// The overflow-policy bucket is deliberately absent: only the handler knows
/// it, so the recording method fills it in. Keeping the two apart stops a
/// caller from labelling a wait with a policy the handler is not using.
#[derive(Clone, Copy)]
pub(crate) struct WaitSite {
    /// Operation name such as `handler_flush`.
    pub(crate) operation: &'static str,
    /// Handler category such as `file` or `timed_rotating`.
    pub(crate) handler_kind: &'static str,
}

impl WaitSite {
    /// Name one Python entry point on one handler class.
    pub(crate) const fn new(operation: &'static str, handler_kind: &'static str) -> Self {
        Self {
            operation,
            handler_kind,
        }
    }
}

/// Bounded context attached to one recorded wait.
#[derive(Clone, Copy)]
pub(crate) struct WaitContext {
    /// Operation name such as `handler_flush`.
    pub(crate) operation: &'static str,
    /// Handler category such as `file` or `timed_rotating`.
    pub(crate) handler_kind: &'static str,
    /// Overflow policy bucket: `drop`, `block`, or `timeout`.
    pub(crate) policy_bucket: &'static str,
}

/// Times one Python-facing wait and records its outcome when it returns.
pub(crate) struct WaitTimer {
    /// Instant captured immediately before the blocking call.
    started: Instant,
}

impl WaitTimer {
    /// Start timing one wait.
    pub(crate) fn start() -> Self {
        Self {
            started: Instant::now(),
        }
    }

    /// Stop timing, record the outcome, and emit the internal event.
    ///
    /// Call this only after the detached section has returned, so the event is
    /// emitted on the interpreter-owning thread and never prompts the tracing
    /// bridge to attach to Python from a GIL-free thread.
    pub(crate) fn finish(
        self,
        diagnostics: &WaitDiagnostics,
        context: WaitContext,
        outcome: WaitOutcome,
    ) {
        let elapsed = self.started.elapsed();
        diagnostics.record(outcome, elapsed);
        emit_event(context, outcome, elapsed);
    }
}

/// Convert a wait duration to the microseconds the counters and events carry.
///
/// Saturating rather than wrapping: a wait longer than about 584 000 years is
/// not meaningful, and reporting `u64::MAX` is better than an apparent small
/// value.
pub(super) fn elapsed_micros(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX)
}

/// Emit the bounded internal tracing event for one wait.
#[cfg(feature = "tracing-compat")]
fn emit_event(context: WaitContext, outcome: WaitOutcome, elapsed: Duration) {
    tracing::trace!(
        target: "femtologging::internal",
        operation = context.operation,
        handler_kind = context.handler_kind,
        policy_bucket = context.policy_bucket,
        wait_outcome = outcome.key(),
        elapsed_us = elapsed_micros(elapsed),
    );
}

/// Keep the bounded event fields live when the tracing feature is off.
///
/// The counters remain the only observable output in that build, but the
/// fields are still part of the event contract, so reference them instead of
/// letting dead-code analysis report the context as unused.
#[cfg(not(feature = "tracing-compat"))]
fn emit_event(context: WaitContext, outcome: WaitOutcome, elapsed: Duration) {
    let _ = (
        context.operation,
        context.handler_kind,
        context.policy_bucket,
        outcome,
        elapsed,
    );
}
