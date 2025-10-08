//! Unit tests for the rotating file handler.

use super::*;
use crate::formatter::DefaultFormatter;
use crate::handlers::file::{
    BuilderOptions, HandlerConfig, OverflowPolicy, RotationStrategy, TestConfig,
};
use crate::log_record::FemtoLogRecord;
use rstest::rstest;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, BufWriter, ErrorKind, Read, Seek, SeekFrom, Write};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::tempdir;

struct EnvVarGuard<'a> {
    key: &'a str,
}

impl<'a> EnvVarGuard<'a> {
    fn set(key: &'a str, value: &str) -> Self {
        env::set_var(key, value);
        Self { key }
    }
}

impl Drop for EnvVarGuard<'_> {
    fn drop(&mut self) {
        env::remove_var(self.key);
    }
}

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

#[rstest]
#[case::rotates_when_existing_file_and_next_record_exceed_budget(
    "012345678901234567890123456789",
    "",
    "next",
    34,
    true,
    1
)]
#[case::stays_below_threshold("012345678901234567890123456789", "", "next", 35, false, 1)]
#[case::counts_buffered_bytes("seed\n", "pending", "next", 15, true, 1)]
#[case::buffered_fits_exactly("seed\n", "pending", "next", 17, false, 1)]
#[case::multibyte_overflows("", "", "😀", 4, true, 1)]
#[case::multibyte_boundary("", "", "😀", 5, false, 1)]
#[case::single_record_exceeds_limit("", "", "toolong", 5, true, 1)]
#[case::rotation_disabled("", "", "message", 0, false, 0)]
fn rotation_predicate_respects_byte_lengths(
    #[case] initial: &str,
    #[case] buffered: &str,
    #[case] message: &str,
    #[case] max_bytes: u64,
    #[case] should_rotate: bool,
    #[case] backup_count: usize,
) -> io::Result<()> {
    let dir = tempdir()?;
    let path = dir.path().join("rotating.log");
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)?;
    file.write_all(initial.as_bytes())?;
    file.flush()?;
    drop(file);

    let file = OpenOptions::new().read(true).write(true).open(&path)?;
    let mut writer = BufWriter::new(file);
    writer.write_all(buffered.as_bytes())?;

    let mut strategy = FileRotationStrategy::new(path.clone(), max_bytes, backup_count);
    let next_bytes = FileRotationStrategy::next_record_bytes(message);
    assert_eq!(
        strategy.should_rotate(&writer, next_bytes)?,
        should_rotate,
        "rotation decision mismatch for message {message:?}"
    );

    if should_rotate {
        strategy.rotate(&mut writer)?;
        writer.flush()?;
        let mut reopened = OpenOptions::new().read(true).open(&path)?;
        let mut contents = String::new();
        reopened.read_to_string(&mut contents)?;
        assert!(contents.is_empty(), "rotated file should be truncated");
    }

    Ok(())
}

#[rstest]
fn rotate_promotes_existing_backups() -> io::Result<()> {
    let dir = tempdir()?;
    let path = dir.path().join("rotating.log");
    fs::write(path.with_extension("log.1"), "old backup")?;
    fs::write(&path, "seed")?;

    let file = OpenOptions::new().read(true).write(true).open(&path)?;
    let mut writer = BufWriter::new(file);
    let mut strategy = FileRotationStrategy::new(path.clone(), 1, 2);
    strategy.rotate(&mut writer)?;
    writer.flush()?;

    let promoted = fs::read_to_string(path.with_extension("log.2"))?;
    assert_eq!(promoted, "old backup");
    let newest = fs::read_to_string(path.with_extension("log.1"))?;
    assert_eq!(newest, "seed");

    Ok(())
}

#[test]
fn rotation_truncates_in_place_when_no_backups() -> io::Result<()> {
    let dir = tempdir()?;
    let path = dir.path().join("rotating.log");

    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)?;
    let mut writer = BufWriter::new(file);
    writer.write_all(b"before\n")?;
    writer.flush()?;

    let mut reader = OpenOptions::new().read(true).open(&path)?;
    let mut strategy = FileRotationStrategy::new(path.clone(), 1, 0);
    strategy.rotate(&mut writer)?;

    writer.write_all(b"after\n")?;
    writer.flush()?;

    reader.seek(SeekFrom::Start(0))?;
    let mut observed = String::new();
    reader.read_to_string(&mut observed)?;
    assert_eq!(observed, "after\n");

    Ok(())
}

#[rstest]
fn rotate_prunes_excess_backups_when_limit_lowered() -> io::Result<()> {
    let dir = tempdir()?;
    let path = dir.path().join("rotating.log");
    fs::write(path.with_extension("log.1"), "keep")?;
    fs::write(path.with_extension("log.2"), "prune one")?;
    fs::write(path.with_extension("log.3"), "prune two")?;
    fs::write(&path, "seed")?;

    let file = OpenOptions::new().read(true).write(true).open(&path)?;
    let mut writer = BufWriter::new(file);
    let mut strategy = FileRotationStrategy::new(path.clone(), 1, 1);
    strategy.rotate(&mut writer)?;
    writer.flush()?;

    assert!(!path.with_extension("log.2").exists());
    assert!(!path.with_extension("log.3").exists());
    let newest = fs::read_to_string(path.with_extension("log.1"))?;
    assert_eq!(newest, "seed");

    Ok(())
}

#[rstest]
fn rotating_handler_performs_size_based_rotation() -> io::Result<()> {
    let dir = tempdir()?;
    let path = dir.path().join("rotating.log");
    let handler = FemtoRotatingFileHandler::with_capacity_flush_policy(
        &path,
        DefaultFormatter,
        HandlerConfig::default(),
        RotationConfig::new(20, 2),
    )?;
    handler.handle(FemtoLogRecord::new("core", "INFO", "first"));
    handler.handle(FemtoLogRecord::new("core", "INFO", "second"));
    drop(handler);

    let primary = fs::read_to_string(&path)?;
    assert!(primary.contains("second"));
    let backup = path.with_extension("log.1");
    assert!(backup.exists(), "expected first backup file");
    let backup_contents = fs::read_to_string(backup)?;
    assert!(backup_contents.contains("first"));

    Ok(())
}

#[test]
fn before_write_reports_rotation_outcome() -> io::Result<()> {
    let dir = tempdir()?;
    let path = dir.path().join("rotating.log");
    fs::write(&path, "seed\n")?;
    let file = OpenOptions::new().read(true).write(true).open(&path)?;
    let mut writer = BufWriter::new(file);
    let mut strategy = FileRotationStrategy::new(path.clone(), 6, 1);

    let rotated = strategy.before_write(&mut writer, "x")?;
    assert!(rotated, "first append should trigger rotation");

    writer.write_all(b"x\n")?;
    writer.flush()?;

    let rotated = strategy.before_write(&mut writer, "ok")?;
    assert!(
        !rotated,
        "second append should not rotate once log is empty"
    );

    Ok(())
}

#[test]
fn rotate_falls_back_to_append_when_reopen_fails() -> io::Result<()> {
    let dir = tempdir()?;
    let path = dir.path().join("rotating.log");
    fs::write(&path, "seed\n")?;
    let file = OpenOptions::new().read(true).write(true).open(&path)?;
    let mut writer = BufWriter::new(file);
    let mut strategy = FileRotationStrategy::new(path.clone(), 1, 1);

    let _guard = EnvVarGuard::set("FEMTOLOGGING_FORCE_ROTATE_FRESH_FAILURE", "once");
    let err = strategy
        .rotate(&mut writer)
        .expect_err("fresh open should fail");
    assert_eq!(err.kind(), io::ErrorKind::Other);

    writer.write_all(b"after\n")?;
    writer.flush()?;

    let backup = strategy.backup_path(1);
    assert_eq!(fs::read_to_string(&backup)?, "seed\n");
    assert_eq!(fs::read_to_string(&path)?, "after\n");

    Ok(())
}

#[test]
fn rotate_restores_writer_when_backup_rename_fails() -> io::Result<()> {
    let dir = tempdir()?;
    let path = dir.path().join("rotating.log");
    fs::write(&path, "seed\n")?;

    let conflicting = path.with_extension("log.1");
    fs::create_dir(&conflicting)?;

    let file = OpenOptions::new().read(true).write(true).open(&path)?;
    let mut writer = BufWriter::new(file);
    let mut strategy = FileRotationStrategy::new(path.clone(), 1, 1);

    let err = strategy
        .rotate(&mut writer)
        .expect_err("rename conflict should fail rotation");
    assert_ne!(err.kind(), io::ErrorKind::NotFound);

    writer.write_all(b"after\n")?;
    writer.flush()?;

    let contents = fs::read_to_string(&path)?;
    assert!(
        contents.ends_with("after\n"),
        "log should still receive writes after failed rotation: {contents:?}"
    );
    assert!(
        conflicting.is_dir(),
        "conflicting directory should remain after failed rename"
    );

    Ok(())
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
        "producing records should remain fast even while rotation is pending"
    );

    assert!(handler.flush());
    handler.close();

    let recorded = rotations.lock().expect("rotation observer lock poisoned");
    assert!(
        recorded.len() >= 1,
        "expected the rotation observer to capture at least one rotation"
    );

    Ok(())
}

#[rstest]
fn rotating_handler_respects_test_builder_defaults() {
    struct NoopWriter;
    impl Write for NoopWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Seek for NoopWriter {
        fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
            Err(io::Error::new(
                ErrorKind::Unsupported,
                "seek unsupported for NoopWriter",
            ))
        }
    }

    let mut cfg = TestConfig::new(NoopWriter, DefaultFormatter);
    cfg.capacity = 2;
    cfg.flush_interval = 1;

    let handler = FemtoFileHandler::with_writer_for_test(cfg);
    handler.handle(FemtoLogRecord::new("core", "INFO", "message"));
    drop(handler);
}
