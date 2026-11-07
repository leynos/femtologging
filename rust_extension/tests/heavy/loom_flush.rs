//! Loom models for flush barrier ordering and handler flushing.
//!
//! A flush has two promises to keep, and both are orderings rather than
//! values. The drain barrier must be acknowledged only after every record
//! queued ahead of it has been handled, so a flush that returns cannot leave a
//! record for the same logger behind. And the handlers must be flushed only
//! after that acknowledgement, so a handler's `flush` observes every record the
//! caller logged before it.
//!
//! The models vary how many records are queued ahead of the flush — including
//! none, which is the fast path a record-free logger takes — and whether the
//! attached handler reports success or failure. The failing case checks that a
//! handler failure is reported as a failed flush, and still only after the
//! barrier.
//!
//! What the models cannot reach is Python-specific: the GIL-detached wait and
//! delivery (`Python::detach`/`attach`) are compiled only with the `python`
//! feature, and the manager-lock release before a bridge flush is likewise
//! Python-only. The Loom lane builds with `--no-default-features`, so those
//! arms do not exist here. They are covered by the Python regression tests in
//! `tests/test_logger_dispatch.py`, `tests/test_file_handler.py`, and
//! `rust_extension/src/log_compat/flush_tests.rs`; this model covers the
//! ordering the record and handler paths share with them.

use std::any::Any;
use std::sync::Arc;

use loom::sync::{Arc as LoomArc, Mutex as LoomMutex};

use _femtologging_rs::{FemtoHandlerTrait, FemtoLevel, FemtoLogRecord, FemtoLogger, HandlerError};

/// Record- and flush-observation state shared with the asserting model thread.
struct ProbeState {
    /// Number of records the worker has delivered to this handler.
    deliveries: usize,
    /// Number of times `flush` has been called.
    flushes: usize,
    /// Deliveries observed when `flush` was first called, if it ever was.
    deliveries_at_first_flush: Option<usize>,
}

/// Handler that records delivery and flush ordering in Loom-protected state.
struct FlushProbe {
    /// Shared observation state.
    state: LoomArc<LoomMutex<ProbeState>>,
    /// Result every `flush` call reports.
    flush_succeeds: bool,
}

impl FlushProbe {
    /// Create a probe whose flush reports *flush_succeeds*.
    fn new(flush_succeeds: bool) -> Self {
        Self {
            state: LoomArc::new(LoomMutex::new(ProbeState {
                deliveries: 0,
                flushes: 0,
                deliveries_at_first_flush: None,
            })),
            flush_succeeds,
        }
    }

    /// Copy the observed counts out of the shared state.
    fn snapshot(&self) -> (usize, usize, Option<usize>) {
        let state = self
            .state
            .lock()
            .expect("flush probe state should not be poisoned");
        (
            state.deliveries,
            state.flushes,
            state.deliveries_at_first_flush,
        )
    }
}

impl FemtoHandlerTrait for FlushProbe {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        self.state
            .lock()
            .expect("flush probe state should not be poisoned")
            .deliveries += 1;
        Ok(())
    }

    fn flush(&self) -> bool {
        let mut state = self
            .state
            .lock()
            .expect("flush probe state should not be poisoned");
        let deliveries = state.deliveries;
        state.deliveries_at_first_flush.get_or_insert(deliveries);
        state.flushes += 1;
        self.flush_succeeds
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Run one model with *record_count* records queued before the flush.
fn run_flush_model(record_count: usize, flush_succeeds: bool) {
    loom::model(move || {
        let probe = Arc::new(FlushProbe::new(flush_succeeds));
        let logger =
            FemtoLogger::new("flush".to_string()).expect("test logger should be constructible");
        logger
            .add_handler(probe.clone() as Arc<dyn FemtoHandlerTrait>)
            .expect("native flush probe should register");

        for index in 0..record_count {
            let _ = logger.log(FemtoLevel::Info, &format!("record-{index}"));
        }

        let flushed = logger.flush_handlers();
        let (deliveries, flushes, deliveries_at_first_flush) = probe.snapshot();

        assert_eq!(
            flushed, flush_succeeds,
            "the flush result must report the handler's own verdict"
        );
        assert_eq!(
            deliveries, record_count,
            "a flush must not return before every queued record is handled"
        );
        assert_eq!(
            flushes, 1,
            "an attached handler must be flushed exactly once per flush"
        );
        assert_eq!(
            deliveries_at_first_flush,
            Some(record_count),
            "a handler flush must observe every record logged before it"
        );

        drop(probe);
        drop(logger);
    });
}

/// The drain barrier is acknowledged before the handler flush, for every
/// queued-record count and handler outcome.
#[test]
fn loom_flush_barrier_orders_records_before_handler_flush() {
    for record_count in 0..=2 {
        for flush_succeeds in [true, false] {
            run_flush_model(record_count, flush_succeeds);
        }
    }
}
