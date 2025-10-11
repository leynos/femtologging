//! Concurrency-focused tests for the rotating handler.

use super::super::*;
use crate::formatter::DefaultFormatter;
use crate::handlers::file::{
    BuilderOptions, HandlerConfig, OverflowPolicy, RotationStrategy,
};
use crate::log_record::FemtoLogRecord;
use std::fs::OpenOptions;
use std::io::{self, BufWriter, Write};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
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
            self.rotations
                .lock()
                .expect("rotation observer lock poisoned")
                .push(thread::current().id());
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[test]
fn rotation_runs_on_worker_thread() -> io::Result<()> {
    let dir = tempdir()?;
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
        .open(&path)?;
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
    handler.handle(FemtoLogRecord::new("core", "INFO", "alpha"));
    handler.handle(FemtoLogRecord::new("core", "INFO", "beta"));
    assert!(handler.flush());
    handler.close();

    let recorded = rotations.lock().expect("rotation observer lock poisoned");
    assert!(
        !recorded.is_empty(),
        "expected at least one rotation to be recorded"
    );
    assert!(recorded.iter().all(|id| *id != producer_id));

    Ok(())
}

#[test]
fn rotation_keeps_producers_non_blocking() -> io::Result<()> {
    let dir = tempdir()?;
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
        .open(&path)?;
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

    handler.handle(FemtoLogRecord::new("core", "INFO", "seed"));
    handler.handle(FemtoLogRecord::new("core", "INFO", "trigger"));

    let wait_start = Instant::now();
    while !started.load(Ordering::SeqCst) {
        if wait_start.elapsed() > Duration::from_secs(2) {
            panic!("rotation did not begin within the expected time window");
        }
        thread::sleep(Duration::from_millis(1));
    }

    let start = Instant::now();
    for idx in 0..8 {
        handler.handle(FemtoLogRecord::new("core", "INFO", &format!("extra {idx}")));
    }
    assert!(
        start.elapsed() < Duration::from_millis(200),
        "additional writes must not block while rotation is in progress"
    );

    handler.close();
    let recorded = rotations.lock().expect("rotation observer lock poisoned");
    assert!(
        !recorded.is_empty(),
        "expected rotation to complete while producers kept writing"
    );

    Ok(())
}
