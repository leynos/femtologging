//! Tests verifying `FemtoLogger::drop` releases the handle mutex
//! before joining the worker thread.

use super::logger_tests_helpers::HandlePtr;
use super::*;

fn setup_logger_for_drop_test() -> Result<FemtoLogger, String> {
    let mut logger = FemtoLogger::new("drop-lock".to_string());
    if let Some(shutdown_tx) = logger.shutdown_tx.take() {
        let _ = shutdown_tx.send(());
    }
    logger.tx.take();
    let original_handle = { logger.handle.lock().take() };
    if let Some(handle) = original_handle {
        handle
            .join()
            .map_err(|_| "initial worker thread panicked".to_string())?;
    }
    Ok(logger)
}

fn spawn_lock_attempt_worker(
    handle_ptr: HandlePtr,
    start_signal: std::sync::mpsc::Receiver<()>,
    result_tx: std::sync::mpsc::Sender<bool>,
    release_signal: std::sync::mpsc::Receiver<()>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        use std::time::{Duration, Instant};

        // A closed channel means the test has already failed, so the worker
        // exits quietly and the test times out with context.
        if start_signal.recv().is_err() {
            return;
        }
        // SAFETY: The logger outlives the worker, and the mutex guards access.
        let handle_mutex = unsafe { handle_ptr.as_ref() };
        let start = Instant::now();
        let mut acquired = false;
        while start.elapsed() < Duration::from_millis(200) {
            if let Some(_guard) = handle_mutex.try_lock() {
                acquired = true;
                break;
            }
            std::thread::yield_now();
        }
        let _ = result_tx.send(acquired);
        let _ = release_signal.recv();
    })
}

fn wait_for_drop_to_acquire_lock(handle_ptr: HandlePtr, timeout: std::time::Duration) {
    use std::time::Instant;

    // SAFETY: The logger outlives the drop thread and mutex guards access.
    let handle_mutex = unsafe { handle_ptr.as_ref() };
    let probe_start = Instant::now();
    while probe_start.elapsed() < timeout {
        if let Some(guard) = handle_mutex.try_lock() {
            if guard.is_none() {
                return;
            }
        }
        std::thread::yield_now();
    }
    panic!("Timed out waiting for drop thread to take handle mutex");
}

#[test]
fn drop_releases_handle_lock_before_join() {
    use std::sync::mpsc;
    use std::time::Duration;

    let logger =
        Box::new(setup_logger_for_drop_test().expect("drop-test logger arrangement must succeed"));

    let handle_ptr = HandlePtr(&logger.handle as *const Mutex<Option<std::thread::JoinHandle<()>>>);

    let (start_lock_tx, start_lock_rx) = mpsc::channel();
    let (attempt_done_tx, attempt_done_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (drop_started_tx, drop_started_rx) = mpsc::channel();

    let worker_handle =
        spawn_lock_attempt_worker(handle_ptr, start_lock_rx, attempt_done_tx, release_rx);

    *logger.handle.lock() = Some(worker_handle);

    let drop_thread = std::thread::spawn(move || {
        drop_started_tx
            .send(())
            .expect("Failed to signal drop start");
        drop(logger);
    });

    drop_started_rx
        .recv()
        .expect("Failed to wait for drop start");
    wait_for_drop_to_acquire_lock(handle_ptr, Duration::from_millis(200));

    start_lock_tx
        .send(())
        .expect("Failed to start lock attempt");
    let acquired = attempt_done_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("Failed to receive lock attempt result");
    release_tx.send(()).expect("Failed to release worker");
    drop_thread.join().expect("Drop thread panicked");

    assert!(
        acquired,
        "expected drop to release handle mutex before joining"
    );
}
