//! Transaction-guard safety tests for runtime configuration.
//!
//! Every configuration entry point holds the GIL *and* the process-wide runtime
//! transaction guard. Two properties follow, and both are easy to break because
//! the failure mode is a hang rather than a wrong value:
//!
//! 1. Acquiring the guard must never block while the GIL is held, or a peer
//!    thread waiting for the GIL can never finish the transaction that would
//!    release it.
//! 2. Definition owners displaced by a commit must not be dropped until the
//!    guard is released. Dropping the last Python reference can run a
//!    `__del__` that re-enters configuration and waits for the same,
//!    non-reentrant guard on the same thread.
//!
//! These tests use timeouts throughout: a regression here deadlocks, so a
//! missing assertion would show up as a hung suite instead of a failure.
#![cfg(all(test, feature = "python"))]

use std::{
    any::Any,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use pyo3::Python;
use rstest::rstest;
use serial_test::serial;

use super::{
    ConfigBuilder, DeferredDrop, LoggerConfigBuilder, LoggerMutationBuilder, RuntimeConfigBuilder,
    test_utils::gil_and_clean_manager,
};
use crate::{
    FemtoLevel,
    filters::{FemtoFilter, FilterContext, FilterDecision},
    handler::{FemtoHandlerTrait, HandlerError},
    log_record::FemtoLogRecord,
    logger::{DetachedOwners, HandlerAttachment},
    manager,
};

/// Upper bound for a transaction that must not block. Generous enough for a
/// busy host, short enough that a deadlock fails instead of hanging the suite.
const TRANSACTION_TIMEOUT: Duration = Duration::from_secs(30);

/// How many probe filter drops have run, and how many of those saw the guard taken.
static FILTERS_RELEASED: AtomicUsize = AtomicUsize::new(0);
/// Count of probe filter drops that observed the transaction guard still held.
static FILTERS_RELEASED_UNDER_GUARD: AtomicUsize = AtomicUsize::new(0);
/// Count of probe handler drops.
static HANDLERS_RELEASED: AtomicUsize = AtomicUsize::new(0);
/// How many re-entrant configuration attempts completed.
static REENTERED: AtomicUsize = AtomicUsize::new(0);

/// Test filter whose `Drop` records the guard state at release time.
///
/// This is the invariant the deferred drop exists to protect: a Python finalizer
/// cannot re-enter configuration safely if the guard is still held. Observing the
/// guard directly makes the assertion independent of reference counts, which are
/// easy to perturb from the test itself.
struct GuardProbeFilter;

impl FemtoFilter for GuardProbeFilter {
    fn decision(
        &self,
        _record: &mut FemtoLogRecord,
        _context: &mut FilterContext,
    ) -> FilterDecision {
        FilterDecision::accept(true)
    }
}

impl Drop for GuardProbeFilter {
    fn drop(&mut self) {
        // `parking_lot` mutexes are not reentrant, so a guard still held by this
        // thread also reports `None` here rather than deadlocking the probe.
        if super::try_lock_runtime_transaction().is_none() {
            FILTERS_RELEASED_UNDER_GUARD.fetch_add(1, Ordering::SeqCst);
        }
        FILTERS_RELEASED.fetch_add(1, Ordering::SeqCst);
    }
}

/// Test handler that counts its own releases.
struct ProbeHandler;

impl FemtoHandlerTrait for ProbeHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Drop for ProbeHandler {
    fn drop(&mut self) {
        HANDLERS_RELEASED.fetch_add(1, Ordering::SeqCst);
    }
}

/// Filter that runs a nested configuration transaction from its own drop.
///
/// A Python `__del__` that calls `apply()` reaches exactly this code path: the
/// release happens with the GIL held, and the nested call needs the same
/// process-wide guard. Expressing the hazard in Rust keeps the test independent
/// of how a callback object happens to be freed, while still covering the
/// ordering that matters.
struct ReenteringFilter;

impl FemtoFilter for ReenteringFilter {
    fn decision(
        &self,
        _record: &mut FemtoLogRecord,
        _context: &mut FilterContext,
    ) -> FilterDecision {
        FilterDecision::accept(true)
    }
}

impl Drop for ReenteringFilter {
    fn drop(&mut self) {
        RuntimeConfigBuilder::new()
            .with_logger(
                "core",
                LoggerMutationBuilder::new().with_level(FemtoLevel::Warn),
            )
            .apply()
            .expect("a drop that re-enters configuration must not deadlock");
        REENTERED.fetch_add(1, Ordering::SeqCst);
    }
}

/// Build a baseline logger holding a handler and a level filter.
fn install_baseline() {
    ConfigBuilder::new()
        .with_filter(
            "lvl",
            crate::filters::FilterBuilder::Level(
                crate::filters::LevelFilterBuilder::new().with_max_level(FemtoLevel::Debug),
            ),
        )
        .with_handler("stderr", crate::StreamHandlerBuilder::stderr())
        .with_root_logger(LoggerConfigBuilder::new().with_level(FemtoLevel::Debug))
        .with_logger(
            "core",
            LoggerConfigBuilder::new()
                .with_handlers(["stderr"])
                .with_filters(["lvl"]),
        )
        .build_and_init()
        .expect("the baseline configuration should build");
}

/// Run `body` on its own thread and fail if it does not finish in time.
///
/// The body runs inside `Python::attach` there, because the transaction paths
/// assume a GIL-holding caller and the whole point of these tests is how that
/// thread interacts with the guard. A deadlocked body never signals, so the
/// timeout is what turns a hang into an ordinary test failure.
fn run_with_timeout<F>(what: &str, body: F)
where
    F: FnOnce() + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        // Report completion before unwinding, so a panicking body surfaces as
        // its own message instead of being reported as a timeout.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Python::attach(|_| body());
        }));
        let _ = tx.send(outcome.is_ok());
    });
    match rx.recv_timeout(TRANSACTION_TIMEOUT) {
        Ok(true) => handle
            .join()
            .expect("the transaction thread must not panic"),
        Ok(false) => panic!("{what} panicked; see the panic message above"),
        Err(error) => panic!("{what} did not finish within {TRANSACTION_TIMEOUT:?}: {error}"),
    }
}

/// A second transaction completes while a first has already released the guard.
///
/// This is the ordinary serialization property, and it guards the polling
/// acquisition against a regression that never releases the guard.
#[rstest]
#[serial]
fn sequential_transactions_both_complete(_gil_and_clean_manager: ()) {
    run_with_timeout("two sequential transactions", || {
        install_baseline();
        for _ in 0..2 {
            RuntimeConfigBuilder::new()
                .with_logger(
                    "core",
                    LoggerMutationBuilder::new().with_level(FemtoLevel::Info),
                )
                .apply()
                .expect("each sequential transaction should succeed");
        }
    });
}

/// A filter displaced by a real commit is released only after the guard is free.
///
/// The probe is attached directly to a live logger, so the next transaction
/// displaces it through the ordinary `replace_filters` path. Its `Drop` then
/// observes whether the guard was still held, which is exactly the state a
/// Python finalizer would deadlock in.
#[rstest]
#[serial]
fn displaced_filter_is_released_after_the_transaction(_gil_and_clean_manager: ()) {
    run_with_timeout("a filter replacement", || {
        install_baseline();
        FILTERS_RELEASED.store(0, Ordering::SeqCst);
        FILTERS_RELEASED_UNDER_GUARD.store(0, Ordering::SeqCst);

        Python::attach(|py| {
            let logger = manager::get_logger(py, "core").expect("the core logger should exist");
            logger.borrow(py).add_filter(Arc::new(GuardProbeFilter));
        });
        assert_eq!(
            FILTERS_RELEASED.load(Ordering::SeqCst),
            0,
            "an attached filter must stay alive for the lifetime of its logger",
        );

        RuntimeConfigBuilder::new()
            .with_logger(
                "core",
                LoggerMutationBuilder::new().with_level(FemtoLevel::Info),
            )
            .apply()
            .expect("the replacement transaction should succeed");

        assert_eq!(
            FILTERS_RELEASED.load(Ordering::SeqCst),
            1,
            "the displaced filter should be released exactly once",
        );
        assert_eq!(
            FILTERS_RELEASED_UNDER_GUARD.load(Ordering::SeqCst),
            0,
            "the displaced filter must not be released while the guard is held",
        );
    });
}

/// A drop that re-enters configuration does not deadlock the applying thread.
///
/// This is the regression that motivates the deferred drop. If the guard were
/// still held while displaced owners were released, the nested `apply()` below
/// would wait on a guard its own thread already owns and never return.
#[rstest]
#[serial]
fn reentering_drop_can_apply_another_transaction(_gil_and_clean_manager: ()) {
    run_with_timeout("a re-entering drop", || {
        install_baseline();
        REENTERED.store(0, Ordering::SeqCst);

        Python::attach(|py| {
            let logger = manager::get_logger(py, "core").expect("the core logger should exist");
            logger.borrow(py).add_filter(Arc::new(ReenteringFilter));
        });

        RuntimeConfigBuilder::new()
            .with_logger(
                "core",
                LoggerMutationBuilder::new().with_level(FemtoLevel::Info),
            )
            .apply()
            .expect("the outer transaction should succeed");

        assert_eq!(
            REENTERED.load(Ordering::SeqCst),
            1,
            "the displaced filter must be released once the guard is free, so its \
             nested transaction can run",
        );
    });
}

/// Handler attachments displaced by a commit are released after publication.
///
/// Mirrors the filter case for the other owner kind, so a future change that
/// defers only one of the two is caught. The test drops its own snapshot handle
/// before applying, so the only remaining references are the ones the
/// transaction itself releases.
#[rstest]
#[serial]
fn displaced_handlers_are_released_after_the_transaction(_gil_and_clean_manager: ()) {
    run_with_timeout("a handler replacement", || {
        install_baseline();
        let committed = manager::snapshot_runtime_state();
        let handler = committed
            .handler_registry
            .get("stderr")
            .cloned()
            .expect("the baseline handler should be published");
        drop(committed);
        let weak = Arc::downgrade(&handler);

        RuntimeConfigBuilder::new()
            .with_logger("core", LoggerMutationBuilder::new().clear_handlers())
            .apply()
            .expect("clearing handlers should succeed");

        drop(handler);
        assert!(
            weak.upgrade().is_none(),
            "a cleared handler must be released once the transaction ends",
        );
    });
}

/// The deferred-drop bundle releases handler owners, not just filters.
#[rstest]
#[serial]
fn deferred_drop_releases_handler_attachments() {
    HANDLERS_RELEASED.store(0, Ordering::SeqCst);

    let handler: Arc<dyn FemtoHandlerTrait> = Arc::new(ProbeHandler);
    let mut bundle = DeferredDrop::default();
    bundle.detached.push(DetachedOwners {
        attachments: vec![HandlerAttachment::new(handler)],
        filters: Vec::new(),
    });

    assert_eq!(
        HANDLERS_RELEASED.load(Ordering::SeqCst),
        0,
        "holding the bundle must keep the attachment alive",
    );
    bundle.release();
    assert_eq!(
        HANDLERS_RELEASED.load(Ordering::SeqCst),
        1,
        "releasing the bundle must drop the held handler exactly once",
    );
}
