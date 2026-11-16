//! Behavioural tests for the rotating strategy.

use crate::formatter::DefaultFormatter;
use crate::handler::FemtoHandlerTrait;
use crate::handlers::file::{FemtoFileHandler, HandlerConfig, TestConfig};
use crate::handlers::rotating::strategy::FileRotationStrategy;
use crate::handlers::rotating::{FemtoRotatingFileHandler, RotationConfig};
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;
use rstest::rstest;
use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use tempfile::tempdir;

/// Inputs and expectation for one size-based rotation predicate case.
struct RotationCase {
    initial: &'static str,
    buffered: &'static str,
    message: &'static str,
    max_bytes: u64,
    should_rotate: bool,
    backup_count: usize,
}

#[rstest]
#[case::rotates_when_existing_file_and_next_record_exceed_budget(RotationCase {
    initial: "012345678901234567890123456789",
    buffered: "",
    message: "next",
    max_bytes: 34,
    should_rotate: true,
    backup_count: 1,
})]
#[case::stays_below_threshold(RotationCase {
    initial: "012345678901234567890123456789",
    buffered: "",
    message: "next",
    max_bytes: 35,
    should_rotate: false,
    backup_count: 1,
})]
#[case::counts_buffered_bytes(RotationCase {
    initial: "seed\n",
    buffered: "pending",
    message: "next",
    max_bytes: 15,
    should_rotate: true,
    backup_count: 1,
})]
#[case::buffered_fits_exactly(RotationCase {
    initial: "seed\n",
    buffered: "pending",
    message: "next",
    max_bytes: 17,
    should_rotate: false,
    backup_count: 1,
})]
#[case::multibyte_overflows(RotationCase {
    initial: "",
    buffered: "",
    message: "😀",
    max_bytes: 4,
    should_rotate: true,
    backup_count: 1,
})]
#[case::multibyte_boundary(RotationCase {
    initial: "",
    buffered: "",
    message: "😀",
    max_bytes: 5,
    should_rotate: false,
    backup_count: 1,
})]
#[case::single_record_exceeds_limit(RotationCase {
    initial: "",
    buffered: "",
    message: "toolong",
    max_bytes: 5,
    should_rotate: true,
    backup_count: 1,
})]
#[case::rotation_disabled(RotationCase {
    initial: "",
    buffered: "",
    message: "message",
    max_bytes: 0,
    should_rotate: false,
    backup_count: 0,
})]
fn rotation_predicate_respects_byte_lengths(#[case] case: RotationCase) {
    let RotationCase {
        initial,
        buffered,
        message,
        max_bytes,
        should_rotate,
        backup_count,
    } = case;
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
