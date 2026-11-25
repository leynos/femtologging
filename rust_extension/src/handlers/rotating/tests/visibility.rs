//! Helpers that wait for the rotating worker to publish its records.
//!
//! `handle` only queues a record, and both `Drop` and `flush` bound their own
//! waits at one second. A test that read the log files after a drop therefore
//! asserted on output whose arrival it never established. These helpers wait
//! on the observable file state instead, with a budget independent of any
//! single handler deadline, so a loaded host cannot make the assertion a
//! race. [`rotation_waits_past_a_one_second_worker_stall`] pins that property
//! by parking the worker deliberately.

use crate::formatter::{DefaultFormatter, FemtoFormatter};
use crate::handler::FemtoHandlerTrait;
use crate::handlers::file::HandlerConfig;
use crate::handlers::rotating::{FemtoRotatingFileHandler, RotationConfig};
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tempfile::tempdir;

/// How long a behaviour test waits for the worker to publish its records.
///
/// This is far longer than the handler worker's per-record flush interval, so
/// a loaded host has ample time. The wait is on the file contents themselves,
/// not on one `flush()` attempt, so a worker that misses any single
/// one-second flush deadline still lets the test proceed.
pub(super) const RECORD_VISIBILITY_BUDGET: Duration = Duration::from_secs(10);

/// Poll the file at `path` until `is_ready` accepts its contents.
///
/// A missing file counts as not ready yet: the worker creates it when it first
/// rotates or writes.
pub(super) fn wait_until_file_matches<F>(path: &Path, mut is_ready: F)
where
    F: FnMut(&str) -> bool,
{
    let started_at = Instant::now();
    loop {
        if let Ok(contents) = fs::read_to_string(path)
            && is_ready(&contents)
        {
            return;
        }
        assert!(
            started_at.elapsed() < RECORD_VISIBILITY_BUDGET,
            "the worker did not publish the expected contents in {path:?} within \
             {RECORD_VISIBILITY_BUDGET:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// State a [`GatedFormatter`] shares between the worker and the test.
#[derive(Default)]
struct GateState {
    /// Whether the worker has reached the formatter at least once.
    has_entered: bool,
    /// Whether the test has released the worker to continue formatting.
    is_released: bool,
}

/// A formatter that parks the worker until the test releases it.
///
/// The worker formats before it writes, so blocking here stops every queued
/// record from reaching the file. The gate is one-way: once released, later
/// records format without blocking, which is what lets the stall test observe
/// the worker catching up.
#[derive(Clone, Default)]
struct GatedFormatter {
    /// Entered/released flags, guarded by one mutex.
    state: Arc<(Mutex<GateState>, Condvar)>,
}

impl GatedFormatter {
    /// Create a formatter whose gate starts closed.
    fn new() -> Self {
        Self::default()
    }

    /// Mark the worker as parked and block until the test releases it.
    fn wait_until_released(&self) {
        // A poisoned lock still holds flags that remain valid data.
        let (state, released) = &*self.state;
        let mut state = state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.has_entered = true;
        released.notify_all();
        while !state.is_released {
            state = released
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    /// Release the parked worker; later entries pass straight through.
    fn release(&self) {
        let (state, released) = &*self.state;
        let mut state = state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.is_released = true;
        released.notify_all();
    }

    /// Wait up to `timeout` for the worker to reach the formatter.
    fn wait_for_entry(&self, timeout: Duration) {
        let (state, released) = &*self.state;
        let state = state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (state, _) = released
            .wait_timeout_while(state, timeout, |state| !state.has_entered)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(
            state.has_entered,
            "the worker never reached the formatter within {timeout:?}"
        );
    }
}

impl FemtoFormatter for GatedFormatter {
    fn format(&self, record: &FemtoLogRecord) -> String {
        self.wait_until_released();
        DefaultFormatter.format(record)
    }
}

#[test]
fn rotation_waits_past_a_one_second_worker_stall() {
    let dir = tempdir().expect("tempdir must create a temporary directory");
    let path = dir.path().join("stalled.log");
    let formatter = GatedFormatter::new();
    let handler = FemtoRotatingFileHandler::with_capacity_flush_policy(
        &path,
        formatter.clone(),
        HandlerConfig::default(),
        RotationConfig::new(20, 2),
    )
    .expect("rotating handler must be created");
    handler
        .handle(FemtoLogRecord::new("core", FemtoLevel::Info, "first"))
        .expect("first record queued");
    handler
        .handle(FemtoLogRecord::new("core", FemtoLevel::Info, "second"))
        .expect("second record queued");

    // Park the worker inside the formatter for longer than the one-second
    // deadline a single `flush()` call would wait. The wait must not give up
    // while the worker is alive but stalled.
    formatter.wait_for_entry(Duration::from_secs(5));
    std::thread::sleep(Duration::from_millis(1_200));
    formatter.release();

    let backup = path.with_extension("log.1");
    wait_until_file_matches(&path, |primary| primary.contains("second"));
    wait_until_file_matches(&backup, |backup| backup.contains("first"));
    drop(handler);

    let primary = fs::read_to_string(&path).expect("primary log file must be readable");
    assert!(primary.contains("second"));
    let backup_contents = fs::read_to_string(backup).expect("backup file must be readable");
    assert!(backup_contents.contains("first"));
}
