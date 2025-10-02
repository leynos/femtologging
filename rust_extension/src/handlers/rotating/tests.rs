//! Unit tests for the rotating file handler.

use super::*;
use crate::formatter::DefaultFormatter;
use crate::handlers::file::TestConfig;
use crate::log_record::FemtoLogRecord;
use rstest::rstest;
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
