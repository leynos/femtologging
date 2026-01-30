//! Behavioural tests for the rotating strategy.

use super::super::*;
use crate::formatter::DefaultFormatter;
use crate::handlers::file::{FemtoFileHandler, HandlerConfig, RotationStrategy, TestConfig};
use crate::handlers::rotating::strategy::RotationOutcome;
use crate::handlers::rotating::{
    FemtoRotatingFileHandler, RotationConfig, force_fresh_failure_once_for_test,
};
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;
use rstest::rstest;
use serial_test::serial;
use std::fs::{self, OpenOptions};
use std::io::{self, BufWriter, ErrorKind, Read, Seek, SeekFrom, Write};
use tempfile::tempdir;

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
    handler
        .handle(FemtoLogRecord::new("core", FemtoLevel::Info, "first"))
        .expect("first record queued");
    handler
        .handle(FemtoLogRecord::new("core", FemtoLevel::Info, "second"))
        .expect("second record queued");
    drop(handler);

    let primary = fs::read_to_string(&path)?;
    assert!(primary.contains("second"));
    let backup = path.with_extension("log.1");
    assert!(backup.exists(), "expected first backup file");
    let backup_contents = fs::read_to_string(backup)?;
    assert!(backup_contents.contains("first"));

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
    handler
        .handle(FemtoLogRecord::new("core", FemtoLevel::Info, "message"))
        .expect("record queued");
    drop(handler);
}

#[serial(rotating_fresh_failure)]
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
    assert_eq!(
        strategy.take_last_outcome(),
        RotationOutcome::Rotated,
        "rotation must record success outcome"
    );

    writer.write_all(b"x\n")?;
    writer.flush()?;

    let rotated = strategy.before_write(&mut writer, "ok")?;
    assert!(
        !rotated,
        "second append should not rotate once log is empty"
    );
    assert_eq!(
        strategy.take_last_outcome(),
        RotationOutcome::Skipped,
        "subsequent call must record skipped outcome"
    );

    Ok(())
}

#[serial(rotating_fresh_failure)]
#[test]
fn rotate_falls_back_to_append_when_reopen_fails() -> io::Result<()> {
    let dir = tempdir()?;
    let path = dir.path().join("rotating.log");
    fs::write(&path, "seed\n")?;
    let file = OpenOptions::new().read(true).write(true).open(&path)?;
    let mut writer = BufWriter::new(file);
    let mut strategy = FileRotationStrategy::new(path.clone(), 1, 1);

    let _guard = force_fresh_failure_once_for_test("once");
    let rotated = strategy.before_write(&mut writer, "next")?;
    assert!(
        rotated,
        "rotation must proceed even when fresh reopen fails"
    );
    match strategy.take_last_outcome() {
        RotationOutcome::RotatedWithAppendFallback { error } => assert_eq!(
            error,
            "simulated fresh writer failure for testing (once)".to_string()
        ),
        other => panic!("unexpected rotation outcome: {other:?}"),
    }

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
        .before_write(&mut writer, "trigger")
        .expect_err("rename conflict should fail rotation");
    assert_ne!(err.kind(), io::ErrorKind::NotFound);
    assert_eq!(
        strategy.take_last_outcome(),
        RotationOutcome::Failed {
            error: err.to_string()
        },
        "failure outcome must record error message"
    );

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
