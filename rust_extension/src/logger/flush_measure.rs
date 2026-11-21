//! Measurement seam for one flush attempt.
//!
//! The flush path depends on these traits rather than on the process-wide
//! counters, so a unit test can observe exactly what one attempt reported
//! without sharing state with every other logger in the process. The production
//! observer is selected at the flush boundary in [`super::flush`]:
//! [`FlushMetrics`] and the tracing span are the two implementations, and the
//! process-wide instance lives here behind them so orchestration never reaches
//! for an ambient global.

use std::sync::OnceLock;
use std::time::Duration;

use crate::monotonic_clock::MonotonicClock;

#[cfg(feature = "tracing-compat")]
use super::flush_metrics;
use super::flush_metrics::{FlushMetrics, FlushOutcome};

/// Receives the measurement of one completed flush attempt.
///
/// The flush path depends on this trait rather than on the process-wide
/// counters, so a unit test can observe exactly what one attempt reported
/// without sharing state with every other logger in the process.
pub(crate) trait FlushObserver {
    /// Record the terminal `outcome` of a flush that took `elapsed`.
    fn record(&self, outcome: FlushOutcome, elapsed: Duration);
}

/// A terminal outcome the measurement seam is able to record.
///
/// The seam is generic over this so one command measures both vocabularies: a
/// test drives it with a plain [`FlushOutcome`], while production hands it the
/// [`BarrierOutcome`] it also has to return, and only the counters see the
/// four-bucket collapse.
///
/// [`BarrierOutcome`]: super::flush::BarrierOutcome
pub(crate) trait MeasuredOutcome {
    /// Return the metric bucket this outcome belongs to.
    fn metric_outcome(&self) -> FlushOutcome;
}

impl MeasuredOutcome for FlushOutcome {
    fn metric_outcome(&self) -> FlushOutcome {
        *self
    }
}

/// Time one flush attempt and report its outcome to `observer`.
///
/// This is the whole command: one attempt, one clock reading either side of
/// it, and one record, so a counter and a duration always describe the same
/// flush. The attempt itself is supplied by the caller, which keeps this
/// function free of any dependency on the shard, the handlers, or the
/// process-wide metrics.
pub(crate) fn measure_flush_attempt<C, O, T>(
    clock: &C,
    observer: &O,
    attempt: impl FnOnce() -> T,
) -> T
where
    C: MonotonicClock + ?Sized,
    O: FlushObserver + ?Sized,
    T: MeasuredOutcome,
{
    let started = clock.now();
    let outcome = attempt();
    observer.record(
        outcome.metric_outcome(),
        clock.now().saturating_duration_since(started),
    );
    outcome
}

/// Observer that counts a flush in one set of flush metrics.
///
/// The target is a parameter rather than the process-wide instance so a test
/// can hold its own counters and attribute a record to it without sharing
/// state with every other logger in the test binary. Production passes
/// [`process_flush_metrics`], which is the only instance with a lifetime long
/// enough to be injected anywhere else.
pub(crate) struct MetricsFlushObserver<'a> {
    /// Counters that receive the recorded outcome and duration.
    metrics: &'a FlushMetrics,
}

impl<'a> MetricsFlushObserver<'a> {
    /// Build an observer that reports to `metrics`.
    pub(crate) const fn new(metrics: &'a FlushMetrics) -> Self {
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
pub(super) fn production_flush_observer() -> impl FlushObserver {
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
