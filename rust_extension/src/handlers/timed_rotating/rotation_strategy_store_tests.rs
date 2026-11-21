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
    path::{Path, PathBuf},
};

use chrono::Duration;
use rstest::{fixture, rstest};
use tempfile::{TempDir, tempdir};

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

/// Timestamped sibling of `app.log` with `base` as its suffix.
///
/// The path is derived from the test's own directory: the scripted store
/// performs every step it is not told to refuse, so pruning really removes
/// these paths, and they must not name a location on the host.
fn backup(directory: &Path, base: &str) -> PathBuf {
    directory.join(format!("app.log.{base}"))
}

/// A rotation strategy that is due on its second write.
///
/// Every failure test needs the same three things — a log path nothing else on
/// the host can see, a seconds schedule, and a clock that reports one interval
/// between the initial write and the write that triggers rotation — so the
/// fixture owns them and gives each test a fresh copy. What a test arranges on
/// top is its own: the records already on disk, the writer it holds, and which
/// store step is refused.
struct DueRotation {
    /// Keeps the temporary directory alive for the test that owns the fixture.
    _directory: TempDir,
    /// Active log path inside the temporary directory.
    path: PathBuf,
    /// Cadence that makes every valid suffix a rotation this handler produces.
    schedule: TimedRotationSchedule,
    /// Clock reporting the setup instant and then one interval later.
    clock: SequenceClock,
}

#[fixture]
fn due_rotation() -> DueRotation {
    let directory = tempdir().expect("tempdir must create a temporary directory");
    let path = directory.path().join("timed.log");
    let schedule = TimedRotationSchedule::new(TimedRotationWhen::Seconds, 1, true, None)
        .expect("seconds schedule must validate");
    let start = utc_datetime!("2026-03-12T00:00:00Z");
    let clock = SequenceClock::new([start, start + Duration::seconds(2)]);
    DueRotation {
        _directory: directory,
        path,
        schedule,
        clock,
    }
}

#[test]
fn retention_prunes_through_the_injected_store() {
    let directory = tempdir().expect("tempdir must create a temporary directory");
    // The listing is deliberately not in chronological order, so a prune that
    // stops sorting would remove the newest backups instead of the oldest.
    let store = ScriptedFileStore::with_listing(&[
        "app.log",
        "app.log.2026-03-12_00-00-02",
        "app.log.notes",
        "app.log.2026-03-12_00-00-00",
        "other.log.2026-03-12_00-00-00",
        "app.log.2026-03-12_00-00-01",
    ]);
    let schedule = TimedRotationSchedule::new(TimedRotationWhen::Seconds, 1, true, None)
        .expect("seconds schedule must validate");
    let retention = BackupRetention::new(directory.path().join("app.log"), 1);

    retention
        .prune(&store, |suffix| schedule.is_valid_suffix(suffix))
        .expect("pruning a listing must succeed");

    assert_removals!(
        store,
        vec![
            backup(directory.path(), "2026-03-12_00-00-00"),
            backup(directory.path(), "2026-03-12_00-00-01"),
        ],
        "only timestamped backups count, and the oldest are pruned first"
    );
}

#[rstest]
fn a_refused_rename_leaves_the_original_writer_in_place(due_rotation: DueRotation) {
    let DueRotation {
        _directory,
        path,
        schedule,
        clock,
    } = due_rotation;
    let store = ScriptedFileStore::with_listing(&[]).failing_at(StoreStep::Rename);
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
        .expect_err("a refused rename must surface as a rotation error");
    assert!(
        error.to_string().contains("scripted Rename failure"),
        "the error must name the refused step rather than an earlier one: {error}"
    );

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

#[rstest]
fn a_refused_fresh_open_restores_the_writer_then_rolls_back_the_rename(due_rotation: DueRotation) {
    let DueRotation {
        _directory,
        path,
        schedule,
        clock,
    } = due_rotation;
    let store = ScriptedFileStore::with_listing(&[]).failing_at(StoreStep::Open);
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

/// A failed rollback must be undone before the next rotation stages.
///
/// The fresh open and the rename back onto the active path both fail, which
/// leaves the timestamped file holding the only copy of the records while the
/// active path is missing. The deadline did not advance, so the next due write
/// retries the rotation, and staging removes the timestamped target before it
/// renames. A retry that begins by staging therefore deletes the records. The
/// strategy must instead restore the active path first, leaving the records
/// readable after the retried rotation.
#[rstest]
fn a_failed_rollback_is_recovered_before_the_next_rotation_stages(due_rotation: DueRotation) {
    let DueRotation {
        _directory,
        path,
        schedule,
        clock,
    } = due_rotation;
    fs::write(&path, "prior records\n").expect("the active log must be writable");
    let store = ScriptedFileStore::with_listing(&[])
        .failing_at(StoreStep::Open)
        .failing_rename_onto(&path);
    let mut strategy = TimedFileRotationStrategy::with_seams(
        path.clone(),
        schedule,
        1,
        RotationSeams::new(clock, store.clone()),
    );
    let mut writer = BufWriter::new(
        OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("log file must open"),
    );

    let error = RotationStrategy::before_write(&mut strategy, &mut writer, "second")
        .expect_err("a failed rollback must surface as a rotation error");
    assert!(
        error.to_string().contains("rollback rename also failed"),
        "the error must name the undo that failed: {error}"
    );
    let rotated = store.renames()[0].1.clone();
    assert!(
        !path.exists(),
        "the failed rollback must leave the active path missing"
    );
    assert_eq!(
        fs::read_to_string(&rotated).expect("the timestamped log must be readable"),
        "prior records\n",
        "the timestamped file must hold the only copy of the records"
    );

    // The worker still writes the triggering record when `before_write`
    // returns an error, so the retried rotation has to carry it as well as the
    // records that were already buffered. Without this write, a retry that
    // left the writer pointing at a stale file could still satisfy the checks
    // below.
    writer
        .write_all(b"triggering record\n")
        .expect("the writer must still accept the triggering record");
    writer.flush().expect("the writer must flush");
    assert_eq!(
        fs::read_to_string(&rotated).expect("the timestamped log must be readable"),
        "prior records\ntriggering record\n",
        "the failed rollback must leave the triggering record in the timestamped file"
    );

    RotationStrategy::before_write(&mut strategy, &mut writer, "third")
        .expect("a retry must restore the active path and rotate");

    let renames = store.renames();
    assert_eq!(
        renames.len(),
        3,
        "the retry must add only the recovery and the forward rename: {renames:?}"
    );
    assert_eq!(
        renames[1],
        (rotated.clone(), path.clone()),
        "the retry must rename the records back onto the active path first"
    );
    assert_eq!(
        fs::read_to_string(&rotated).expect("the rotated log must be readable"),
        "prior records\ntriggering record\n",
        "the retried rotation must not lose the records the retry followed"
    );
}
