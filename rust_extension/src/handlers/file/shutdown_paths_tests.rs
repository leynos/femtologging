//! Behavioural tests for the file handler's shutdown outcomes.
//!
//! A closer can reach three verdicts: the worker finished, the deadline
//! expired, or the worker panicked. The last one is the interesting case,
//! because a closer that no longer owns the join handle still has to report a
//! panic that happened after an earlier closer gave up.

use super::scripted_writer::{ScriptedWriter, handler_with};
use super::wait_outcomes::ShutdownOutcome;
use super::*;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

/// A worker that outlives the close deadline reports a timeout.
#[test]
fn a_blocked_shutdown_reports_a_timeout() {
    let (entered_tx, _entered_rx) = mpsc::channel();
    let writer = ScriptedWriter::blocking(entered_tx);
    let handler = handler_with(writer.clone(), OverflowPolicy::Block);

    assert_eq!(
        handler.take_shutdown().wait(),
        ShutdownOutcome::TimedOut,
        "a worker still flushing at the deadline must report a timeout",
    );

    writer.release();
    assert!(
        handler.completion.wait(Duration::from_secs(2)),
        "the released worker must still signal completion",
    );
}

/// A worker that panics during shutdown is reported as a panic.
#[test]
fn a_panicking_shutdown_reports_a_panic() {
    let (entered_tx, _entered_rx) = mpsc::channel();
    let writer = ScriptedWriter::panicking_flush(entered_tx);
    let handler = handler_with(writer, OverflowPolicy::Block);

    assert_eq!(
        handler.take_shutdown().wait(),
        ShutdownOutcome::Panicked,
        "a worker that panicked must not be reported as a clean exit",
    );
}

/// A closer that no longer owns the join handle still reports a later panic.
#[test]
fn a_later_closer_reports_a_panic_after_an_earlier_timeout() {
    let (entered_tx, _entered_rx) = mpsc::channel();
    let writer = ScriptedWriter::blocking_then_panicking_flush(entered_tx);
    let handler = Arc::new(handler_with(writer.clone(), OverflowPolicy::Block));

    let (taken_tx, taken_rx) = mpsc::channel();
    let (outcome_tx, outcome_rx) = mpsc::channel();
    let first_closer = {
        let handler = Arc::clone(&handler);
        thread::spawn(move || {
            let shutdown = handler.take_shutdown();
            taken_tx
                .send(())
                .expect("taken signal receiver must stay open");
            outcome_tx
                .send(shutdown.wait())
                .expect("outcome receiver must stay open");
        })
    };

    taken_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("the first closer must take the worker state");
    assert_eq!(
        outcome_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("the first closer must report its outcome"),
        ShutdownOutcome::TimedOut,
        "the first closer must time out while the worker is blocked",
    );

    // The worker now panics, after the first closer has already given up and
    // dropped the join handle.
    writer.release();
    assert!(
        handler.completion.wait(Duration::from_secs(2)),
        "the panicking worker must still signal completion",
    );

    assert_eq!(
        handler.take_shutdown().wait(),
        ShutdownOutcome::Panicked,
        "a closer without the join handle must read the worker's verdict",
    );
    first_closer.join().expect("the first closer must finish");
}

/// A closer that sees completion with no recorded verdict reports `Unknown`.
///
/// The worker records its verdict before signalling completion, so this state
/// should not arise in production. It is constructed here directly because the
/// alternative is an untested fallback: a wrong mapping would silently count a
/// clean exit as unknown, or drop the outcome entirely.
#[test]
fn a_closer_without_a_verdict_reports_unknown() {
    let completion = crate::shutdown::ShutdownCompletion::new();
    completion.complete();
    let shutdown = WorkerShutdown::new(
        None,
        Arc::clone(&completion),
        crate::shutdown::TerminalStatus::new(),
    );

    let outcome = shutdown.wait();
    assert_eq!(
        outcome,
        ShutdownOutcome::Unknown,
        "completion without a verdict must not be reported as a clean exit",
    );

    let diagnostics = WaitDiagnostics::new();
    WaitTimer::start().finish(
        &diagnostics,
        WaitContext {
            operation: "handler_close",
            handler_kind: "file",
            policy_bucket: "drop",
        },
        outcome.into(),
    );
    assert_eq!(
        diagnostics.snapshot().count(WaitOutcome::CloseUnknown),
        1,
        "the unknown outcome must reach its own counter",
    );
}
