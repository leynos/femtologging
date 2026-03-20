//! Tests for timed rotation core logic.

use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write};

use chrono::{Duration, NaiveTime};
use rstest::rstest;
use tempfile::tempdir;

use crate::handlers::{
    file::RotationStrategy,
    timed_rotating::{clock::SequenceClock, schedule::TimedRotationWhen},
};

use super::core::TimedFileRotationStrategy;
use super::schedule::TimedRotationSchedule;
use super::test_helpers::utc_datetime;

#[rstest]
fn rotates_and_prunes_backups() {
    let dir = tempdir().expect("tempdir must create a temporary directory");
    let path = dir.path().join("timed.log");
    let notes_path = dir.path().join("timed.log.notes");
    let schedule = TimedRotationSchedule::new(TimedRotationWhen::Seconds, 1, true, None)
        .expect("seconds schedule must validate");
    let start = utc_datetime("2026-03-12T00:00:00Z");
    let clock = SequenceClock::new([
        start,
        start,
        start + Duration::seconds(2),
        start + Duration::seconds(4),
    ]);
    let mut strategy = TimedFileRotationStrategy::new_with_clock(path.clone(), schedule, 1, clock);
    let mut writer = BufWriter::new(
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .expect("log file must open"),
    );

    RotationStrategy::before_write(&mut strategy, &mut writer, "first")
        .expect("initial rollover check must succeed");
    writer
        .write_all(b"first\n")
        .expect("first record must be written");
    writer.flush().expect("first flush must succeed");

    RotationStrategy::before_write(&mut strategy, &mut writer, "second")
        .expect("first rotation must succeed");
    writer
        .write_all(b"second\n")
        .expect("second record must be written");
    writer.flush().expect("second flush must succeed");

    fs::write(&notes_path, "keep me").expect("sibling file must be created");

    RotationStrategy::before_write(&mut strategy, &mut writer, "third")
        .expect("second rotation must succeed");
    writer
        .write_all(b"third\n")
        .expect("third record must be written");
    writer.flush().expect("third flush must succeed");

    assert!(
        notes_path.exists(),
        "non-rotated siblings must not be pruned as backups",
    );
    assert!(
        !path
            .with_file_name("timed.log.2026-03-12_00-00-00")
            .exists(),
        "oldest timed backup must be pruned",
    );
    assert!(
        path.with_file_name("timed.log.2026-03-12_00-00-02")
            .exists(),
        "most recent timed backup must remain",
    );
}

#[rstest]
fn midnight_schedule_is_preserved() {
    let schedule = TimedRotationSchedule::new(
        TimedRotationWhen::Midnight,
        1,
        true,
        Some(NaiveTime::from_hms_opt(0, 0, 0).expect("midnight must be valid")),
    )
    .expect("midnight schedule must validate");

    let next = schedule.next_rollover(utc_datetime("2026-03-11T23:59:59Z"));

    assert_eq!(next, utc_datetime("2026-03-12T00:00:00Z"));
}
