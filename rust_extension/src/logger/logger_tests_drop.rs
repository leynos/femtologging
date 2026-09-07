//! Tests that joining a logger worker releases its handle mutex first.

use super::*;
use parking_lot::Mutex;
use std::sync::Arc;

fn spawn_lock_attempt_worker(
    handle: Arc<Mutex<Option<std::thread::JoinHandle<()>>>>,
    start_signal: std::sync::mpsc::Receiver<()>,
    result_tx: std::sync::mpsc::Sender<bool>,
    release_signal: std::sync::mpsc::Receiver<()>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        use std::time::{Duration, Instant};
        if start_signal.recv().is_err() {
            return;
        }
        let start = Instant::now();
        let mut acquired = false;
        while start.elapsed() < Duration::from_millis(200) {
            if let Some(_guard) = handle.try_lock() {
                acquired = true;
                break;
            }
            std::thread::yield_now();
        }
        if result_tx.send(acquired).is_err() {
            return;
        }
        let Ok(()) = release_signal.recv() else {
            return;
        };
    })
}

fn wait_for_join_worker_to_take_handle(
    handle: &Mutex<Option<std::thread::JoinHandle<()>>>,
    timeout: std::time::Duration,
) -> Result<(), String> {
    use std::time::Instant;
    let probe_start = Instant::now();
    while probe_start.elapsed() < timeout {
        if let Some(guard) = handle.try_lock()
            && guard.is_none()
        {
            return Ok(());
        }
        std::thread::yield_now();
    }
    Err("timed out waiting for join_worker to take the handle".to_owned())
}

#[test]
fn join_worker_releases_handle_lock_before_joining() {
    use std::sync::mpsc;
    use std::time::Duration;
    let handle = Arc::new(Mutex::new(None));
    let (start_lock_tx, start_lock_rx) = mpsc::channel();
    let (attempt_done_tx, attempt_done_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let worker_handle = spawn_lock_attempt_worker(
        Arc::clone(&handle),
        start_lock_rx,
        attempt_done_tx,
        release_rx,
    );
    *handle.lock() = Some(worker_handle);
    let handle_for_join = Arc::clone(&handle);
    let join_thread = std::thread::spawn(move || FemtoLogger::join_worker(&handle_for_join));
    wait_for_join_worker_to_take_handle(&handle, Duration::from_millis(200))
        .expect("join_worker should take the handle before joining");
    start_lock_tx
        .send(())
        .expect("lock-attempt worker should start");
    let acquired = attempt_done_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("lock-attempt worker should report its result");
    drop(release_tx);
    join_thread
        .join()
        .expect("join_worker thread should finish");
    assert!(
        acquired,
        "join_worker must release the handle mutex before joining"
    );
}
