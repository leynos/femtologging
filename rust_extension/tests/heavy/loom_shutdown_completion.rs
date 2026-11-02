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

use loom::thread;

use _femtologging_rs::shutdown::ShutdownCompletion;

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
