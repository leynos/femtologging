//! Synchronous handler flushing for [`super::FemtoLogger`].
//!
//! A flush has two halves: the logger's own queued records must be handled
//! before the handlers are asked to flush, and the handlers must then be
//! flushed without holding any lock a handler could need. Both halves are
//! timed and classified once, so the counters and the tracing span describe
//! the same attempt.
//!
//! The measurement is a seam of its own. [`measure_flush_attempt`] takes the
//! clock and the observer as parameters, so orchestration reads no ambient
//! time source and writes no global; [`FlushMetrics`] and the tracing span are
//! two production observers selected at the flush boundary.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crate::handler::FemtoHandlerTrait;
use crate::monotonic_clock::MonotonicClock;

use super::DrainOutcome;
use super::FemtoLogger;
#[cfg(feature = "tracing-compat")]
use super::flush_metrics;
use super::flush_metrics::{FlushMetrics, FlushOutcome};

impl FemtoLogger {
    /// Flush all handlers attached to this logger.
    ///
    /// This call blocks until the worker acknowledges a flush marker and each
    /// attached handler has flushed. Enqueueing the flush marker and waiting
    /// for its acknowledgement share a two-second timeout. Flushing attached
    /// handlers can take additional time.
    ///
    /// When Python handlers can be attached, callers that currently hold
    /// Python access must detach before calling this method. Otherwise the
    /// worker cannot acquire the GIL to run a Python handler.
    ///
    /// # Returns
    ///
    /// `true` if the worker acknowledges the flush marker within the shared
    /// timeout and every attached handler flush succeeds. `false` if the
    /// marker cannot be enqueued or acknowledged in time, or a handler flush
    /// fails.
    ///
    /// # Examples
    ///
    /// A Rust caller holding Python access detaches during the blocking flush:
    ///
    /// ```
    /// use _femtologging_rs::FemtoLogger;
    /// use pyo3::Python;
    ///
    /// let logger = FemtoLogger::new(String::from("example"))
    ///     .expect("the shared dispatch pool starts");
    /// let flushed = Python::attach(|py| py.detach(|| logger.flush_handlers()));
    /// assert!(flushed);
    /// ```
    pub fn flush_handlers(&self) -> bool {
        #[cfg(all(feature = "python", feature = "test-util"))]
        self.flush_test_hooks.signal_entered();

        self.flush_handlers_blocking()
    }

    /// Drain this logger's queued records, then flush its configured handlers.
    ///
    /// The whole attempt is timed and classified once, so the counters and the
    /// tracing span describe the same flush. The production clock and observer
    /// are chosen here, at the boundary: the orchestration below names neither
    /// the process-wide metrics nor the tracing feature. The observer opens
    /// its span when it is created, before the drain, so an external
    /// subscriber sees the wait as part of the operation.
    pub(super) fn flush_handlers_blocking(&self) -> bool {
        let observer = production_flush_observer();
        let outcome =
            measure_flush_attempt(&crate::monotonic_clock::SystemClock, &observer, || {
                self.flush_handlers_inner()
            });
        outcome.is_success()
    }

    /// Classify one flush attempt by its terminal outcome.
    fn flush_handlers_inner(&self) -> FlushOutcome {
        if let Some(failure) = FlushOutcome::from_drain(self.wait_for_dispatch_to_drain()) {
            return failure;
        }
        if self.flush_configured_handlers() {
            FlushOutcome::Success
        } else {
            FlushOutcome::HandlerFailure
        }
    }

    /// Wait for the logger's shard to finish the records queued ahead.
    ///
    /// The barrier travels the shard's own queue, so it cannot overtake a
    /// record this logger logged first, and a shard that is gone or too slow
    /// reports [`DrainOutcome::QueueClosed`] or [`DrainOutcome::TimedOut`]
    /// rather than waiting indefinitely.
    ///
    /// A logger with nothing in flight has nothing of its own to drain, so it
    /// answers immediately. The budget releases a record's charge only after
    /// its handlers have run, so a count of zero means every record this
    /// logger queued has been handled. Queuing a barrier anyway would cost more
    /// than the round trip: the `log` bridge's `flush_all_handlers` sweeps
    /// every registered logger, and a barrier takes a queue slot but no
    /// permit, so barriers can crowd out the records they were meant to drain
    /// and make `admit` refuse work the shard had room for. `Drop` already
    /// skips its wait this way.
    ///
    /// The wait is taken with the interpreter detached. `flush_handlers` is a
    /// Python method, so its caller holds the GIL, and a queued Python handler
    /// needs that same GIL to finish — holding it here would make the wait time
    /// out against a handler that could not run. Detaching releases the GIL for
    /// the duration, and the wait is taken directly when there is no
    /// interpreter to detach from, which is also when no Python handler can be
    /// mid-call.
    fn wait_for_dispatch_to_drain(&self) -> DrainOutcome {
        #[cfg(not(loom))]
        let (shard, budget) = match self.dispatch_state() {
            Ok(dispatch) => dispatch,
            // A logger whose pool cannot be reached has no shard to drain, so
            // its barrier can never be answered.
            Err(_) => return DrainOutcome::QueueClosed,
        };
        #[cfg(not(loom))]
        if budget.in_flight() == 0 {
            return DrainOutcome::Drained;
        }
        #[cfg(not(loom))]
        if let Some(drained) =
            pyo3::Python::try_attach(|py| py.detach(|| shard.wait_for_drain_outcome()))
        {
            return drained;
        }
        #[cfg(not(loom))]
        return shard.wait_for_drain_outcome();
        #[cfg(loom)]
        self.shard.wait_for_drain_outcome()
    }

    /// Run one real flush attempt with an injected clock and observer.
    ///
    /// Test-only: production selects the clock and the observer at the
    /// boundary in [`Self::flush_handlers_blocking`]. This entry point runs the
    /// same drain-then-flush orchestration, so a test can classify the shard
    /// and the attached handlers through the production path while the
    /// measurement lands in counters the test owns.
    #[cfg(all(test, not(loom)))]
    pub(crate) fn flush_handlers_measured<C, O>(&self, clock: &C, observer: &O) -> FlushOutcome
    where
        C: MonotonicClock + ?Sized,
        O: FlushObserver + ?Sized,
    {
        measure_flush_attempt(clock, observer, || self.flush_handlers_inner())
    }

    /// Flush every configured handler and report whether all succeeded.
    ///
    /// The handler list is snapshotted under the read guard and the guard is
    /// released before any handler runs. A handler's flush may be arbitrary
    /// Python, and Python code is free to log through this logger or to add and
    /// remove handlers; holding the read guard across that call would stall
    /// those writers, and a writer that then waits on the flush would deadlock.
    ///
    /// Every handler is flushed even after one reports failure. A flush is
    /// best-effort: one broken handler must not leave the records buffered in
    /// the handlers after it, so the result is the conjunction of all attempts
    /// rather than the first failure.
    fn flush_configured_handlers(&self) -> bool {
        let handlers: Vec<Arc<dyn FemtoHandlerTrait>> = {
            let attachments = self.handlers.read();
            attachments
                .iter()
                .map(|attachment| Arc::clone(&attachment.handler))
                .collect()
        };
        let mut all_succeeded = true;
        for handler in handlers {
            if !handler.flush() {
                all_succeeded = false;
            }
        }
        all_succeeded
    }
}

/// Receives the measurement of one completed flush attempt.
///
/// The flush path depends on this trait rather than on the process-wide
/// counters, so a unit test can observe exactly what one attempt reported
/// without sharing state with every other logger in the process.
pub(crate) trait FlushObserver {
    /// Record the terminal `outcome` of a flush that took `elapsed`.
    fn record(&self, outcome: FlushOutcome, elapsed: Duration);
}

/// Time one flush attempt and report its outcome to `observer`.
///
/// This is the whole command: one attempt, one clock reading either side of
/// it, and one record, so a counter and a duration always describe the same
/// flush. The attempt itself is supplied by the caller, which keeps this
/// function free of any dependency on the shard, the handlers, or the
/// process-wide metrics.
pub(crate) fn measure_flush_attempt<C, O>(
    clock: &C,
    observer: &O,
    attempt: impl FnOnce() -> FlushOutcome,
) -> FlushOutcome
where
    C: MonotonicClock + ?Sized,
    O: FlushObserver + ?Sized,
{
    let started = clock.now();
    let outcome = attempt();
    observer.record(outcome, clock.now().saturating_duration_since(started));
    outcome
}

/// Observer that counts a flush in one set of flush metrics.
///
/// The target is a parameter rather than the process-wide instance so a test
/// can hold its own counters and attribute a record to it without sharing
/// state with every other logger in the test binary. Production passes
/// [`process_flush_metrics`], which is the only instance with a lifetime long
/// enough to be injected anywhere else.
struct MetricsFlushObserver<'a> {
    /// Counters that receive the recorded outcome and duration.
    metrics: &'a FlushMetrics,
}

impl<'a> MetricsFlushObserver<'a> {
    /// Build an observer that reports to `metrics`.
    const fn new(metrics: &'a FlushMetrics) -> Self {
        Self { metrics }
    }
}

impl FlushObserver for MetricsFlushObserver<'_> {
    fn record(&self, outcome: FlushOutcome, elapsed: Duration) {
        self.metrics.record(outcome, elapsed);
    }
}

/// Observer that records a flush onto a tracing span, then delegates.
#[cfg(feature = "tracing-compat")]
struct TracingFlushObserver<O> {
    /// Span whose fields describe this attempt.
    span: tracing::Span,
    /// Keeps the span entered for the whole attempt.
    _entered: tracing::span::EnteredSpan,
    /// Observer that receives the counters once the span is updated.
    inner: O,
}

#[cfg(feature = "tracing-compat")]
impl<O: FlushObserver> TracingFlushObserver<O> {
    /// Open the flush span now, so a subscriber sees the drain as part of it.
    fn start(inner: O) -> Self {
        let span = tracing::trace_span!(
            target: "femtologging::internal",
            "logger_flush",
            operation = "logger_flush",
            flush_outcome = tracing::field::Empty,
            elapsed_us = tracing::field::Empty,
        );
        let entered = span.clone().entered();
        Self {
            span,
            _entered: entered,
            inner,
        }
    }
}

#[cfg(feature = "tracing-compat")]
impl<O: FlushObserver> FlushObserver for TracingFlushObserver<O> {
    fn record(&self, outcome: FlushOutcome, elapsed: Duration) {
        self.span.record("flush_outcome", outcome.label());
        self.span
            .record("elapsed_us", flush_metrics::elapsed_micros(elapsed));
        self.inner.record(outcome, elapsed);
    }
}

/// The observer a production flush reports to.
///
/// Selected at the flush boundary rather than stored on the logger: the
/// observer is stateless — all its state is the process-wide instance below —
/// so keeping a copy per logger would only widen `FemtoLogger`.
fn production_flush_observer() -> impl FlushObserver {
    #[cfg(feature = "tracing-compat")]
    {
        TracingFlushObserver::start(MetricsFlushObserver::new(process_flush_metrics()))
    }
    #[cfg(not(feature = "tracing-compat"))]
    {
        MetricsFlushObserver::new(process_flush_metrics())
    }
}

/// Return the process-wide flush metrics.
///
/// The instance lives here, behind the production observer, so orchestration
/// and unit tests never reach for an ambient global. The `log` bridge also
/// names it, because a bridge failure is counted without a flush attempt to
/// carry the observer.
pub(crate) fn process_flush_metrics() -> &'static FlushMetrics {
    static METRICS: OnceLock<FlushMetrics> = OnceLock::new();
    METRICS.get_or_init(FlushMetrics::new)
}

/// Record a logger flush that failed during a `log`-bridge flush sweep.
///
/// `metrics` is a parameter so the sweep can be driven against counters a test
/// owns; production passes [`process_flush_metrics`].
#[cfg(feature = "log-compat")]
pub(crate) fn record_bridge_flush_failure(metrics: &FlushMetrics) {
    metrics.record_bridge_failure();
}

#[cfg(all(test, not(loom)))]
#[path = "flush_tests.rs"]
mod tests;
