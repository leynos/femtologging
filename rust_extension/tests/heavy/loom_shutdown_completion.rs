//! Concurrency models for the shared worker-completion signal.
//!
//! A handler's worker can be closed by several threads at once, and a closer
//! may register before the worker finishes, while it finishes, or after it has
//! already finished. These models schedule those interleavings and require
//! every closer to observe the one completion, the signal never to report
//! completion early, and a repeated completion to stay harmless.
//!
//! The timed `wait` is unbounded under Loom, so a model must always complete
//! the signal: otherwise the model blocks instead of failing. The incomplete
//! case is therefore checked through `is_completed`, which never blocks.

use std::{sync::Arc, time::Duration};

use loom::sync::atomic::{AtomicBool, Ordering};
use loom::thread;

use _femtologging_rs::shutdown::{ShutdownCompletion, run_guarded_worker};

/// The timeout passed to `wait` in a model.
///
/// Loom has no clock and ignores the duration; it is present because the
/// production signature requires one.
const MODEL_TIMEOUT: Duration = Duration::from_secs(1);

/// Two closers wait for one worker, which always completes.
///
/// Covers both registration orders: a closer that registered before the
/// worker finished and one that registered afterwards. Every closer must
/// report completion, whichever schedule Loom explores.
#[test]
fn loom_shutdown_completion_wakes_every_concurrent_waiter() {
    loom::model(|| {
        let completion = ShutdownCompletion::new();
        let closers = (0..2)
            .map(|index| {
                let closer_completion = Arc::clone(&completion);
                thread::spawn(move || {
                    assert!(
                        closer_completion.wait(MODEL_TIMEOUT),
                        "closer {index} must observe the worker's completion",
                    );
                })
            })
            .collect::<Vec<_>>();

        completion.complete();

        for closer in closers {
            closer.join().expect("a closer must not panic");
        }
    });
}

/// An unfinished worker never reports completion, and repeats stay harmless.
///
/// The signal starts incomplete, accepts a repeated completion without
/// complaint, and then satisfies a waiter that registers afterwards. The
/// state transition is checked through `is_completed`, which never blocks,
/// because Loom's timed `wait` has no clock to expire against.
#[test]
fn loom_shutdown_completion_reports_only_finished_state() {
    loom::model(|| {
        let completion = ShutdownCompletion::new();
        assert!(
            !completion.is_completed(),
            "an unfinished worker must not report completion",
        );

        completion.complete();
        completion.complete();

        assert!(
            completion.is_completed(),
            "a completed worker must keep reporting completion",
        );
        assert!(
            completion.wait(MODEL_TIMEOUT),
            "a late waiter must return at once after repeated completion",
        );
    });
}

/// Tracks whether one worker-owned resource has been dropped.
///
/// The flag lives behind a Loom `Arc` around a Loom `AtomicBool`, so a closer
/// that reads it after observing completion is checked against the worker's
/// real teardown under every interleaving Loom explores.
#[derive(Clone)]
struct DropFlag(loom::sync::Arc<AtomicBool>);

impl DropFlag {
    /// Create a flag for a resource that is still alive.
    fn new() -> Self {
        Self(loom::sync::Arc::new(AtomicBool::new(false)))
    }

    /// Take a guard whose drop marks the resource as torn down.
    fn guard(&self) -> DropGuard {
        DropGuard(loom::sync::Arc::clone(&self.0))
    }

    /// Report whether the resource has been dropped.
    fn is_dropped(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Marks its resource torn down when dropped, including while unwinding.
struct DropGuard(loom::sync::Arc<AtomicBool>);

impl Drop for DropGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// Spawn a closer that requires both resources to be torn down on completion.
fn spawn_teardown_closer(
    completion: Arc<ShutdownCompletion>,
    writer: DropFlag,
    formatter: DropFlag,
) -> loom::thread::JoinHandle<()> {
    thread::spawn(move || {
        assert!(
            completion.wait(MODEL_TIMEOUT),
            "a closer must observe the worker's completion",
        );
        assert!(
            writer.is_dropped(),
            "completion was reported before the writer was dropped",
        );
        assert!(
            formatter.is_dropped(),
            "completion was reported before the formatter was dropped",
        );
    })
}

/// Completion must follow the worker's writer and formatter teardown.
///
/// The worker body models both resources with drop guards and ends normally.
/// Whichever schedule Loom explores, a closer that observes completion must
/// already see both dropped; a signal raised before teardown fails the model.
#[test]
fn loom_shutdown_completion_follows_writer_and_formatter_teardown() {
    loom::model(|| {
        let completion = ShutdownCompletion::new();
        let writer = DropFlag::new();
        let formatter = DropFlag::new();
        let closer =
            spawn_teardown_closer(Arc::clone(&completion), writer.clone(), formatter.clone());

        run_guarded_worker(Arc::clone(&completion), move || {
            let _writer = writer.guard();
            let _formatter = formatter.guard();
        });

        closer.join().expect("a closer must not panic");
    });
}

/// Teardown ordering must hold when the worker body unwinds.
///
/// A panicking worker never reaches its closing statements, so the guard is
/// what both drops its captures and completes the signal. A closer must still
/// see both resources dropped before completion is reported.
#[test]
fn loom_shutdown_completion_follows_teardown_after_a_worker_panic() {
    loom::model(|| {
        let completion = ShutdownCompletion::new();
        let writer = DropFlag::new();
        let formatter = DropFlag::new();
        let closer =
            spawn_teardown_closer(Arc::clone(&completion), writer.clone(), formatter.clone());

        run_guarded_worker(Arc::clone(&completion), move || {
            let _writer = writer.guard();
            let _formatter = formatter.guard();
            // Unwind without panicking so the model does not run the panic
            // hook once per explored schedule.
            std::panic::resume_unwind(Box::new(()));
        });

        closer.join().expect("a closer must not panic");
    });
}
