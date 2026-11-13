//! Test support for driving one shard's queue from outside this module.
//!
//! A logger-level test needs a shard that no worker serves, so a queued
//! record's permit stays in flight and the flush path must ask the shard to
//! drain. The job type stays private to the parent module, so the receiver
//! and the stand-in thread live here and the caller gets only a handle. This
//! is kept out of `mod.rs` to leave room for the shard itself.

use std::time::Duration;

use super::Shard;

/// How long a stand-in worker waits for its next job.
///
/// Bounded so a test that stops driving the shard fails instead of parking a
/// thread for the rest of the run.
const TEST_WORKER_BOUND: Duration = Duration::from_secs(10);

impl Shard {
    /// Build a shard whose stand-in worker takes the record but never the barrier.
    ///
    /// A logger-level test needs a shard that is alive, so the queued job keeps
    /// its permit in flight, yet will never answer a barrier. The queue's
    /// receiver travels to a stand-in thread here because the job type stays
    /// private to the parent module; the returned handle waits for the record
    /// to be held and releases the thread when it is dropped.
    pub(crate) fn with_unanswering_worker_for_test(depth: usize) -> (Self, TestWorker) {
        let (shard, jobs) = Self::without_worker_for_test(depth);
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let record = jobs
                .recv_timeout(TEST_WORKER_BOUND)
                .expect("the test should queue a record for the stand-in worker");
            let _ = started_tx.send(());
            let barrier = jobs
                .recv_timeout(TEST_WORKER_BOUND)
                .expect("the flush should queue a barrier for the stand-in worker");
            // Dropping the barrier drops its acknowledgement sender without
            // answering, which is what a dead worker looks like to the waiter.
            drop(barrier);
            let _ = release_rx.recv_timeout(TEST_WORKER_BOUND);
            drop(record);
        });
        (
            shard,
            TestWorker {
                started: started_rx,
                release: Some(release_tx),
                handle: Some(handle),
            },
        )
    }
}

/// Handle to the stand-in worker of a shard built for an unanswering test.
///
/// Dropping the handle releases the worker and joins it, so a failed assertion
/// cannot leave a thread holding the queue.
pub(crate) struct TestWorker {
    /// Notified once the worker holds the queued record.
    started: std::sync::mpsc::Receiver<()>,
    /// Releases the worker so it can drop the record.
    release: Option<std::sync::mpsc::Sender<()>>,
    /// The stand-in thread, joined on drop.
    handle: Option<std::thread::JoinHandle<()>>,
}

impl TestWorker {
    /// Block until the stand-in worker holds the queued record.
    ///
    /// Waiting for this before the flush starts is what guarantees the record's
    /// permit is in flight; otherwise the logger could find nothing to drain
    /// and answer without ever asking the shard.
    pub(crate) fn wait_until_record_held(&self, timeout: Duration) {
        self.started
            .recv_timeout(timeout)
            .expect("the stand-in worker should take the queued record");
    }
}

impl Drop for TestWorker {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
