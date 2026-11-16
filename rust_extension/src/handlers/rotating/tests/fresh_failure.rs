//! Tests for fresh-failure recovery during rotation.

use std::fs::{self, OpenOptions};
use std::io::{self, BufWriter, Write};

use serial_test::serial;
use tempfile::tempdir;

use crate::handlers::file::RotationStrategy;
use crate::handlers::rotating::force_fresh_failure_once_for_test;
use crate::handlers::rotating::strategy::{FileRotationStrategy, RotationOutcome};

/// Fails with an I/O error when a test expectation does not hold.
///
/// `#[serial]` erases the `#[test]` attribute for Whitaker's test detection, so
/// these functions are not recognised as test-only code and must report
/// failures with `?` rather than panicking.
fn ensure(condition: bool, message: impl Into<String>) -> io::Result<()> {
    if condition {
        Ok(())
    } else {
        Err(io::Error::other(message.into()))
    }
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
    ensure(rotated, "first append should trigger rotation")?;
    ensure(
        strategy.take_last_outcome() == RotationOutcome::Rotated,
        "rotation must record success outcome",
    )?;

    writer.write_all(b"x\n")?;
    writer.flush()?;

    let rotated_again = strategy.before_write(&mut writer, "ok")?;
    ensure(
        !rotated_again,
        "second append should not rotate once log is empty",
    )?;
    ensure(
        strategy.take_last_outcome() == RotationOutcome::Skipped,
        "subsequent call must record skipped outcome",
    )?;

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
    ensure(
        rotated,
        "rotation must proceed even when fresh reopen fails",
    )?;
    match strategy.take_last_outcome() {
        RotationOutcome::RotatedWithAppendFallback { error } => ensure(
            error == "simulated fresh writer failure for testing (once)",
            format!("unexpected fallback error: {error:?}"),
        )?,
        other => {
            return Err(io::Error::other(format!(
                "unexpected rotation outcome: {other:?}"
            )));
        }
    }

    writer.write_all(b"after\n")?;
    writer.flush()?;

    let backup = strategy.backup_path(1);
    let backup_contents = fs::read_to_string(&backup)?;
    ensure(
        backup_contents == "seed\n",
        format!("backup file must retain the pre-rotation contents, got {backup_contents:?}"),
    )?;
    let primary_contents = fs::read_to_string(&path)?;
    ensure(
        primary_contents == "after\n",
        format!("primary log must receive post-rotation writes, got {primary_contents:?}"),
    )?;

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
    ensure(
        err.kind() != io::ErrorKind::NotFound,
        format!("unexpected error kind: {:?}", err.kind()),
    )?;
    let outcome = strategy.take_last_outcome();
    ensure(
        outcome
            == RotationOutcome::Failed {
                error: err.to_string(),
            },
        format!("failure outcome must record the rename error, got {outcome:?}"),
    )?;

    writer.write_all(b"after\n")?;
    writer.flush()?;

    let contents = fs::read_to_string(&path)?;
    ensure(
        contents.ends_with("after\n"),
        format!("log should still receive writes after failed rotation: {contents:?}"),
    )?;
    ensure(
        conflicting.is_dir(),
        "conflicting directory should remain after failed rename",
    )?;

    Ok(())
}
