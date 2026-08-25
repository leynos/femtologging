//! Concurrency-focused tests for the rotating handler.

use crate::formatter::DefaultFormatter;
use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::handlers::file::{
    BuilderOptions, FemtoFileHandler, HandlerConfig, OverflowPolicy, RotationStrategy,
};
use crate::handlers::rotating::FemtoRotatingFileHandler;
use crate::handlers::rotating::strategy::FileRotationStrategy;
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;
use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::tempdir;

struct ObservedStrategy {
    inner: FileRotationStrategy,
    rotations: Arc<Mutex<Vec<thread::ThreadId>>>,
    rotation_delay: Option<Duration>,
    rotation_started: Option<Arc<AtomicBool>>,
}

impl ObservedStrategy {
    fn new(inner: FileRotationStrategy, rotations: Arc<Mutex<Vec<thread::ThreadId>>>) -> Self {
        Self {
            inner,
            rotations,
            rotation_delay: None,
            rotation_started: None,
        }
    }

    fn with_delay(
        inner: FileRotationStrategy,
        rotations: Arc<Mutex<Vec<thread::ThreadId>>>,
        delay: Duration,
        started: Arc<AtomicBool>,
    ) -> Self {
        Self {
            inner,
            rotations,
            rotation_delay: Some(delay),
            rotation_started: Some(started),
        }
    }
}

impl RotationStrategy<BufWriter<File>> for ObservedStrategy {
    fn before_write(&mut self, writer: &mut BufWriter<File>, formatted: &str) -> io::Result<bool> {
        let next_bytes = FileRotationStrategy::next_record_bytes(formatted);
        if self.inner.should_rotate(writer, next_bytes)? {
            if let Some(flag) = &self.rotation_started {
                flag.store(true, Ordering::SeqCst);
            }
            if let Some(delay) = self.rotation_delay {
                thread::sleep(delay);
            }
            self.inner.rotate(writer)?;
            let mut rotations = match self.rotations.lock() {
                Ok(rotations) => rotations,
                // Preserve observations from the worker even if a prior test
                // assertion panicked while inspecting them.
                Err(poisoned) => poisoned.into_inner(),
            };
            rotations.push(thread::current().id());
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Waits for the delayed rotation strategy to signal that rotation has begun.
fn wait_for_rotation_start(flag: &AtomicBool, timeout: Duration) -> Result<(), &'static str> {
    let started_at = Instant::now();
    while !flag.load(Ordering::SeqCst) {
        if started_at.elapsed() > timeout {
            return Err("rotation did not begin within the expected time window");
        }
        thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

#[test]
fn wait_for_rotation_start_returns_timeout_error() {
    let started = AtomicBool::new(false);
    let result = wait_for_rotation_start(&started, Duration::ZERO);

    assert!(
        result.is_err(),
        "waiting for a rotation that never starts should return an error"
    );
}

/// Queues records without blocking and returns unexpected handler failures.
fn attempt_non_blocking_writes(
    handler: &FemtoRotatingFileHandler,
    count: usize,
) -> Result<Duration, HandlerError> {
    let started_at = Instant::now();
    for idx in 0..count {
        match handler.handle(FemtoLogRecord::new(
            "core",
            FemtoLevel::Info,
            &format!("extra {idx}"),
        )) {
            Ok(()) => {}
            Err(HandlerError::QueueFull) => {
                // Dropped records are acceptable here because the test exercises non-blocking queueing.
            }
            Err(other) => return Err(other),
        }
    }
    Ok(started_at.elapsed())
}

#[test]
fn rotation_runs_on_worker_thread() {
    let dir = tempdir().expect("tempdir must create a temporary directory");
    let path = dir.path().join("worker.log");
    let rotations = Arc::new(Mutex::new(Vec::new()));
    let strategy = ObservedStrategy::new(
        FileRotationStrategy::new(path.clone(), 24, 1),
        Arc::clone(&rotations),
    );
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(true)
        .open(&path)
        .expect("log file must open");
    let writer = BufWriter::new(file);
    let handler_cfg = HandlerConfig {
        capacity: 4,
        flush_interval: 1,
        overflow_policy: OverflowPolicy::Drop,
    };
    let options = BuilderOptions::new(strategy, None);
    let inner = FemtoFileHandler::build_from_worker(writer, DefaultFormatter, handler_cfg, options);
    let mut handler = FemtoRotatingFileHandler::new_with_rotation_limits(inner, 24, 1);

    let producer_id = thread::current().id();
    handler
        .handle(FemtoLogRecord::new("core", FemtoLevel::Info, "alpha"))
        .expect("initial record should be written");
    handler
        .handle(FemtoLogRecord::new("core", FemtoLevel::Info, "beta"))
        .expect("second record should be written");
    assert!(handler.flush());
    handler.close();

    let recorded = rotations.lock().expect("rotation observer lock poisoned");
    assert!(
        !recorded.is_empty(),
        "expected at least one rotation to be recorded"
    );
    assert!(recorded.iter().all(|id| *id != producer_id));
}

#[test]
fn rotation_keeps_producers_non_blocking() {
    let dir = tempdir().expect("tempdir must create a temporary directory");
    let path = dir.path().join("non_blocking.log");
    let rotations = Arc::new(Mutex::new(Vec::new()));
    let started = Arc::new(AtomicBool::new(false));
    let strategy = ObservedStrategy::with_delay(
        FileRotationStrategy::new(path.clone(), 20, 1),
        Arc::clone(&rotations),
        Duration::from_millis(100),
        Arc::clone(&started),
    );
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(true)
        .open(&path)
        .expect("log file must open");
    let writer = BufWriter::new(file);
    let handler_cfg = HandlerConfig {
        capacity: 2,
        flush_interval: 1,
        overflow_policy: OverflowPolicy::Drop,
    };
    let options = BuilderOptions::new(strategy, None);
    let mut handler = FemtoRotatingFileHandler::new_with_rotation_limits(
        FemtoFileHandler::build_from_worker(writer, DefaultFormatter, handler_cfg, options),
        20,
        1,
    );

    handler
        .handle(FemtoLogRecord::new("core", FemtoLevel::Info, "seed"))
        .expect("seed record should be written");
    handler
        .handle(FemtoLogRecord::new("core", FemtoLevel::Info, "trigger"))
        .expect("trigger record should be written to trigger rotation");

    wait_for_rotation_start(&started, Duration::from_secs(2))
        .expect("rotation should begin within the expected time window");

    let elapsed = attempt_non_blocking_writes(&handler, 8)
        .expect("writes should only fail with queue-full during rotation");
    assert!(
        elapsed < Duration::from_millis(200),
        "additional writes must not block while rotation is in progress"
    );

    handler.close();
    let recorded = rotations.lock().expect("rotation observer lock poisoned");
    assert!(
        !recorded.is_empty(),
        "expected rotation to complete while producers kept writing"
    );
}
