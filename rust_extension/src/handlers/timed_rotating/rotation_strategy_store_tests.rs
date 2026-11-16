//! Rotation failure paths and retention, driven through the injected store.
//!
//! Each scenario here arranges the outcome it needs instead of provoking it on
//! disk: retention reads a listing supplied from memory, and the failure paths
//! make one named step refuse. The clock is injected for the same reason, so a
//! rollover is due without waiting for a real interval to elapse.
//!
//! What the tests assert is the handler's recovery contract. A refused rename
//! must leave the original writer in place, and a refused fresh open must first
//! restore the writer and then undo the rename that already moved the file.

use std::{
    fs::{self, OpenOptions},
    io::{BufWriter, Write},
    path::PathBuf,
};

use chrono::Duration;
use tempfile::tempdir;

use super::{
    backup_retention::BackupRetention,
    clock::SequenceClock,
    rotation_store::double::{ScriptedFileStore, StoreStep},
    rotation_strategy::{RotationSeams, TimedFileRotationStrategy},
    schedule::{TimedRotationSchedule, TimedRotationWhen},
    test_helpers::utc_datetime,
};
use crate::handlers::file::RotationStrategy;

/// Assert the paths a scripted store was asked to remove, in order.
///
/// A macro rather than a helper so a failure points at the calling test.
macro_rules! assert_removals {
    ($store:expr, $expected:expr, $reason:expr) => {
        assert_eq!($store.removed(), $expected, "{}", $reason)
    };
}

/// Timestamped siblings of `app.log` share its name and a dot.
fn backup(base: &str) -> PathBuf {
    PathBuf::from(format!("/logs/app.log.{base}"))
}

#[test]
fn retention_prunes_through_the_injected_store() {
    let store = ScriptedFileStore::with_listing(&[
        "app.log",
        "app.log.2026-03-12_00-00-00",
        "app.log.2026-03-12_00-00-01",
        "app.log.2026-03-12_00-00-02",
        "app.log.notes",
        "other.log.2026-03-12_00-00-00",
    ]);
    let schedule = TimedRotationSchedule::new(TimedRotationWhen::Seconds, 1, true, None)
        .expect("seconds schedule must validate");
    let retention = BackupRetention::new(PathBuf::from("/logs/app.log"), 1);

    retention
        .prune(&store, |suffix| schedule.is_valid_suffix(suffix))
        .expect("pruning a listing must succeed");

    assert_removals!(
        store,
        vec![backup("2026-03-12_00-00-00"), backup("2026-03-12_00-00-01")],
        "only timestamped backups count, and the oldest are pruned first"
    );
}

#[test]
fn a_refused_rename_leaves_the_original_writer_in_place() {
    let directory = tempdir().expect("tempdir must create a temporary directory");
    let path = directory.path().join("timed.log");
    let store = ScriptedFileStore::with_listing(&[]).failing_at(StoreStep::Rename);
    let schedule = TimedRotationSchedule::new(TimedRotationWhen::Seconds, 1, true, None)
        .expect("seconds schedule must validate");
    let start = utc_datetime!("2026-03-12T00:00:00Z");
    let clock = SequenceClock::new([start, start + Duration::seconds(2)]);
    let mut strategy = TimedFileRotationStrategy::with_seams(
        path.clone(),
        schedule,
        1,
        RotationSeams::new(clock, store.clone()),
    );
    let mut writer = BufWriter::new(
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .expect("log file must open"),
    );

    RotationStrategy::before_write(&mut strategy, &mut writer, "second")
        .expect_err("a refused rename must surface as a rotation error");

    assert!(
        store.renames().is_empty(),
        "the refused rename must not be recorded as performed"
    );
    writer
        .write_all(b"after failure\n")
        .expect("the restored writer must still accept records");
    writer.flush().expect("the restored writer must flush");
    assert_eq!(
        fs::read_to_string(&path).expect("the active log must be readable"),
        "after failure\n",
        "records written after a refused rotation must reach the active file"
    );
}

#[test]
fn a_refused_fresh_open_restores_the_writer_then_rolls_back_the_rename() {
    let directory = tempdir().expect("tempdir must create a temporary directory");
    let path = directory.path().join("timed.log");
    let store = ScriptedFileStore::with_listing(&[]).failing_at(StoreStep::Open);
    let schedule = TimedRotationSchedule::new(TimedRotationWhen::Seconds, 1, true, None)
        .expect("seconds schedule must validate");
    let start = utc_datetime!("2026-03-12T00:00:00Z");
    let clock = SequenceClock::new([start, start + Duration::seconds(2)]);
    let mut strategy = TimedFileRotationStrategy::with_seams(
        path.clone(),
        schedule,
        1,
        RotationSeams::new(clock, store.clone()),
    );
    let mut writer = BufWriter::new(
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .expect("log file must open"),
    );

    let error = RotationStrategy::before_write(&mut strategy, &mut writer, "second")
        .expect_err("a refused fresh open must surface as a rotation error");

    assert!(
        error.to_string().contains("scripted Open failure"),
        "a rollback that succeeds must report the open failure as the cause: {error}"
    );
    let renames = store.renames();
    assert_eq!(
        renames.len(),
        2,
        "the forward rename and its rollback must both be attempted"
    );
    let (active, rotated) = (&renames[0].0, &renames[0].1);
    assert_eq!(
        (&renames[1].0, &renames[1].1),
        (rotated, active),
        "the rollback must rename the rotated file back onto the active path"
    );
    writer
        .write_all(b"after failure\n")
        .expect("the restored writer must still accept records");
    writer.flush().expect("the restored writer must flush");
    assert_eq!(
        fs::read_to_string(&path).expect("the active log must be readable"),
        "after failure\n",
        "a rolled-back rotation must leave the active file usable"
    );
}
