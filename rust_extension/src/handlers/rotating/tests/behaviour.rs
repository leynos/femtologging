//! Behavioural tests for the rotating strategy.

use std::{
    fs::{self, OpenOptions},
    io::{self, BufWriter, Read, Seek, SeekFrom, Write},
};

use rstest::rstest;
use serial_test::serial;
use tempfile::tempdir;

use crate::{
    formatter::DefaultFormatter,
    handler::FemtoHandlerTrait,
    handlers::{
        file::{FemtoFileHandler, HandlerConfig, RotationStrategy, TestConfig},
        rotating::{
            FemtoRotatingFileHandler,
            RotationConfig,
            force_fresh_failure_once_for_test,
            strategy::{FileRotationStrategy, RotationOutcome},
        },
    },
    level::FemtoLevel,
    log_record::FemtoLogRecord,
};

struct RotationPredicateCase {
    initial: &'static str,
    buffered: &'static str,
    message: &'static str,
    max_bytes: u64,
    should_rotate: bool,
    backup_count: usize,
}

#[rstest]
#[case::rotates_when_existing_file_and_next_record_exceed_budget(RotationPredicateCase { initial: "012345678901234567890123456789", buffered: "", message: "next", max_bytes: 34, should_rotate: true, backup_count: 1 })]
#[case::stays_below_threshold(RotationPredicateCase { initial: "012345678901234567890123456789", buffered: "", message: "next", max_bytes: 35, should_rotate: false, backup_count: 1 })]
#[case::counts_buffered_bytes(RotationPredicateCase { initial: "seed\n", buffered: "pending", message: "next", max_bytes: 15, should_rotate: true, backup_count: 1 })]
#[case::buffered_fits_exactly(RotationPredicateCase { initial: "seed\n", buffered: "pending", message: "next", max_bytes: 17, should_rotate: false, backup_count: 1 })]
#[case::multibyte_overflows(RotationPredicateCase { initial: "", buffered: "", message: "😀", max_bytes: 4, should_rotate: true, backup_count: 1 })]
#[case::multibyte_boundary(RotationPredicateCase { initial: "", buffered: "", message: "😀", max_bytes: 5, should_rotate: false, backup_count: 1 })]
#[case::single_record_exceeds_limit(RotationPredicateCase { initial: "", buffered: "", message: "toolong", max_bytes: 5, should_rotate: true, backup_count: 1 })]
#[case::rotation_disabled(RotationPredicateCase { initial: "", buffered: "", message: "message", max_bytes: 0, should_rotate: false, backup_count: 0 })]
fn rotation_predicate_respects_byte_lengths(#[case] test_case: RotationPredicateCase) {
    let RotationPredicateCase {
        initial,
        buffered,
        message,
        max_bytes,
        should_rotate,
        backup_count,
    } = test_case;
    let dir = tempdir().expect("tempdir must create a temporary directory");
    let path = dir.path().join("rotating.log");
    let mut seed_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
        .expect("log file must open for initial seed content");
    seed_file
        .write_all(initial.as_bytes())
        .expect("initial seed content must be written");
    seed_file.flush().expect("initial seed content must flush");
    drop(seed_file);

    let reopened_file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("log file must reopen for buffered content");
    let mut writer = BufWriter::new(reopened_file);
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

    let first_rotated = strategy.before_write(&mut writer, "x")?;
    if !first_rotated || strategy.take_last_outcome() != RotationOutcome::Rotated {
        return Err(io::Error::other(
            "first append should record a successful rotation",
        ));
    }

    writer.write_all(b"x\n")?;
    writer.flush()?;

    let second_rotated = strategy.before_write(&mut writer, "ok")?;
    if second_rotated || strategy.take_last_outcome() != RotationOutcome::Skipped {
        return Err(io::Error::other(
            "second append should record a skipped rotation",
        ));
    }
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
    if !strategy.before_write(&mut writer, "next")? {
        return Err(io::Error::other(
            "rotation must proceed after fresh reopen failure",
        ));
    }
    match strategy.take_last_outcome() {
        RotationOutcome::RotatedWithAppendFallback { error }
            if error == "simulated fresh writer failure for testing (once)" => {}
        outcome => {
            return Err(io::Error::other(format!(
                "unexpected rotation outcome: {outcome:?}"
            )));
        }
    }

    writer.write_all(b"after\n")?;
    writer.flush()?;
    let backup = strategy.backup_path(1);
    if fs::read_to_string(&backup)? != "seed\n" || fs::read_to_string(&path)? != "after\n" {
        return Err(io::Error::other(
            "fallback rotation did not preserve expected files",
        ));
    }
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

    let Err(err) = strategy.before_write(&mut writer, "trigger") else {
        return Err(io::Error::other("rename conflict should fail rotation"));
    };
    if err.kind() == io::ErrorKind::NotFound
        || strategy.take_last_outcome()
            != (RotationOutcome::Failed {
                error: err.to_string(),
            })
    {
        return Err(io::Error::other(
            "failure outcome should record the rename error",
        ));
    }
    writer.write_all(b"after\n")?;
    writer.flush()?;
    let contents = fs::read_to_string(&path)?;
    if !contents.ends_with("after\n") || !conflicting.is_dir() {
        return Err(io::Error::other(
            "writer should remain usable after failed rotation",
        ));
    }
    Ok(())
}
