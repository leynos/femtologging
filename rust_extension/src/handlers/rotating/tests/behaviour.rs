//! Behavioural tests for the rotating strategy.

use crate::formatter::DefaultFormatter;
use crate::handler::FemtoHandlerTrait;
use crate::handlers::file::{FemtoFileHandler, HandlerConfig, RotationStrategy, TestConfig};
use crate::handlers::rotating::strategy::{FileRotationStrategy, RotationOutcome};
use crate::handlers::rotating::{
    FemtoRotatingFileHandler, RotationConfig, force_fresh_failure_once_for_test,
};
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;
use rstest::rstest;
use serial_test::serial;
use std::fs::{self, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
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
) {
    let dir = tempdir().expect("tempdir must create a temporary directory");
    let path = dir.path().join("rotating.log");
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
        .expect("log file must open for initial seed content");
    file.write_all(initial.as_bytes())
        .expect("initial seed content must be written");
    file.flush().expect("initial seed content must flush");
    drop(file);

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("log file must reopen for buffered content");
    let mut writer = BufWriter::new(file);
    writer
        .write_all(buffered.as_bytes())
        .expect("buffered content must be written");

    let mut strategy = FileRotationStrategy::new(path.clone(), max_bytes, backup_count);
    let next_bytes = FileRotationStrategy::next_record_bytes(message);
    assert_eq!(
        strategy
            .should_rotate(&writer, next_bytes)
            .expect("rotation predicate must succeed"),
        should_rotate,
        "rotation decision mismatch for message {message:?}"
    );

    if should_rotate {
        strategy.rotate(&mut writer).expect("rotation must succeed");
        writer.flush().expect("post-rotation flush must succeed");
        let mut reopened = OpenOptions::new()
            .read(true)
            .open(&path)
            .expect("rotated log file must reopen for reading");
        let mut contents = String::new();
        reopened
            .read_to_string(&mut contents)
            .expect("rotated log file must be readable");
        assert!(contents.is_empty(), "rotated file should be truncated");
    }
}

#[rstest]
fn rotate_promotes_existing_backups() {
    let dir = tempdir().expect("tempdir must create a temporary directory");
    let path = dir.path().join("rotating.log");
    fs::write(path.with_extension("log.1"), "old backup")
        .expect("existing backup file must be seeded");
    fs::write(&path, "seed").expect("primary log file must be seeded");

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("primary log file must open");
    let mut writer = BufWriter::new(file);
    let mut strategy = FileRotationStrategy::new(path.clone(), 1, 2);
    strategy.rotate(&mut writer).expect("rotation must succeed");
    writer.flush().expect("post-rotation flush must succeed");

    let promoted = fs::read_to_string(path.with_extension("log.2"))
        .expect("promoted backup file must be readable");
    assert_eq!(promoted, "old backup");
    let newest = fs::read_to_string(path.with_extension("log.1"))
        .expect("newest backup file must be readable");
    assert_eq!(newest, "seed");
}

#[test]
fn rotation_truncates_in_place_when_no_backups() {
    let dir = tempdir().expect("tempdir must create a temporary directory");
    let path = dir.path().join("rotating.log");

    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
        .expect("log file must open for seed content");
    let mut writer = BufWriter::new(file);
    writer
        .write_all(b"before\n")
        .expect("seed content must be written");
    writer.flush().expect("seed content must flush");

    let mut reader = OpenOptions::new()
        .read(true)
        .open(&path)
        .expect("log file must open for reading");
    let mut strategy = FileRotationStrategy::new(path.clone(), 1, 0);
    strategy.rotate(&mut writer).expect("rotation must succeed");

    writer
        .write_all(b"after\n")
        .expect("post-rotation content must be written");
    writer.flush().expect("post-rotation content must flush");

    reader
        .seek(SeekFrom::Start(0))
        .expect("reader must seek back to the start");
    let mut observed = String::new();
    reader
        .read_to_string(&mut observed)
        .expect("rotated log file must be readable");
    assert_eq!(observed, "after\n");
}

#[rstest]
fn rotate_prunes_excess_backups_when_limit_lowered() {
    let dir = tempdir().expect("tempdir must create a temporary directory");
    let path = dir.path().join("rotating.log");
    fs::write(path.with_extension("log.1"), "keep").expect("backup 1 must be seeded");
    fs::write(path.with_extension("log.2"), "prune one").expect("backup 2 must be seeded");
    fs::write(path.with_extension("log.3"), "prune two").expect("backup 3 must be seeded");
    fs::write(&path, "seed").expect("primary log file must be seeded");

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("primary log file must open");
    let mut writer = BufWriter::new(file);
    let mut strategy = FileRotationStrategy::new(path.clone(), 1, 1);
    strategy.rotate(&mut writer).expect("rotation must succeed");
    writer.flush().expect("post-rotation flush must succeed");

    assert!(!path.with_extension("log.2").exists());
    assert!(!path.with_extension("log.3").exists());
    let newest = fs::read_to_string(path.with_extension("log.1"))
        .expect("newest backup file must be readable");
    assert_eq!(newest, "seed");
}

#[rstest]
fn rotating_handler_performs_size_based_rotation() {
    let dir = tempdir().expect("tempdir must create a temporary directory");
    let path = dir.path().join("rotating.log");
    let handler = FemtoRotatingFileHandler::with_capacity_flush_policy(
        &path,
        DefaultFormatter,
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
    drop(handler);

    let primary = fs::read_to_string(&path).expect("primary log file must be readable");
    assert!(primary.contains("second"));
    let backup = path.with_extension("log.1");
    assert!(backup.exists(), "expected first backup file");
    let backup_contents = fs::read_to_string(backup).expect("backup file must be readable");
    assert!(backup_contents.contains("first"));
}

#[rstest]
fn rotating_handler_respects_test_builder_defaults() {
    let dir = tempdir().expect("tempdir must create a temporary directory");
    let path = dir.path().join("builder_defaults.log");
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .read(true)
        .truncate(true)
        .open(&path)
        .expect("log file must open");

    let mut cfg = TestConfig::new(file, DefaultFormatter);
    cfg.capacity = 2;
    cfg.flush_interval = 1;

    let handler = FemtoFileHandler::with_writer_for_test(cfg);
    handler
        .handle(FemtoLogRecord::new("core", FemtoLevel::Info, "message"))
        .expect("record queued");
    drop(handler);

    let contents = fs::read_to_string(&path).expect("log file must be readable");
    assert_eq!(
        contents, "core [INFO] message\n",
        "default builder options (NoRotation) must write the record without rotating"
    );
    assert!(
        !path.with_extension("log.1").exists(),
        "default builder options must not create a rotated backup file"
    );
}

// `#[serial]` erases the `#[test]` attribute for Whitaker's test detection,
// so these tests are not recognized as test-only code and must propagate
// errors with `?` rather than `.expect(...)`.
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

// `#[serial]` erases the `#[test]` attribute for Whitaker's test detection,
// so this test is not recognized as test-only code and must propagate
// errors with `?` rather than `.expect(...)`.
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
