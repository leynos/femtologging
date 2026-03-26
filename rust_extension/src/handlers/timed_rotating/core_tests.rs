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
#[case::prunes_with_backup_count_1(
    1,
    true,
    false,
    true,
    "non-rotated siblings must not be pruned as backups",
    "oldest timed backup must be pruned",
    "most recent timed backup must remain"
)]
#[case::retains_all_with_backup_count_0(
    0,
    false,
    true,
    true,
    "",
    "first timed backup must be retained when backup_count is zero",
    "second timed backup must be retained when backup_count is zero"
)]
fn rotation_and_pruning_behavior(
    #[case] backup_count: usize,
    #[case] create_notes_file: bool,
    #[case] expect_oldest_exists: bool,
    #[case] expect_recent_exists: bool,
    #[case] notes_assertion_msg: &str,
    #[case] oldest_assertion_msg: &str,
    #[case] recent_assertion_msg: &str,
) {
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
    let mut strategy =
        TimedFileRotationStrategy::new_with_clock(path.clone(), schedule, backup_count, clock);
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

    if create_notes_file {
        fs::write(&notes_path, "keep me").expect("sibling file must be created");
    }

    RotationStrategy::before_write(&mut strategy, &mut writer, "third")
        .expect("second rotation must succeed");
    writer
        .write_all(b"third\n")
        .expect("third record must be written");
    writer.flush().expect("third flush must succeed");

    if create_notes_file {
        assert!(notes_path.exists(), "{notes_assertion_msg}");
    }

    let oldest_path = path.with_file_name("timed.log.2026-03-12_00-00-00");
    assert_eq!(
        oldest_path.exists(),
        expect_oldest_exists,
        "{oldest_assertion_msg}"
    );

    let recent_path = path.with_file_name("timed.log.2026-03-12_00-00-02");
    assert_eq!(
        recent_path.exists(),
        expect_recent_exists,
        "{recent_assertion_msg}"
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

#[rstest]
fn seed_rollover_from_overrides_initial_rollover() {
    let dir = tempdir().expect("tempdir must create a temporary directory");
    let path = dir.path().join("timed.log");
    let schedule = TimedRotationSchedule::new(TimedRotationWhen::Hours, 1, true, None)
        .expect("hourly schedule must validate");

    let now = utc_datetime("2026-03-12T10:00:00Z");
    let mtime = utc_datetime("2026-03-12T08:00:00Z");
    let clock = SequenceClock::new([now]);
    let mut strategy = TimedFileRotationStrategy::new_with_clock(path, schedule.clone(), 1, clock);

    strategy.seed_rollover_from(mtime);

    let expected = schedule.next_rollover(mtime);
    let not_expected = schedule.next_rollover(now);
    assert_eq!(
        strategy.next_rollover_at(),
        expected,
        "next_rollover_at must be reseeded from mtime"
    );
    assert_ne!(
        strategy.next_rollover_at(),
        not_expected,
        "next_rollover_at must not retain the original clock seed"
    );
}

#[rstest]
fn new_with_clock_uses_clock_time() {
    let dir = tempdir().expect("tempdir must create a temporary directory");
    let path = dir.path().join("timed.log");
    let schedule = TimedRotationSchedule::new(TimedRotationWhen::Hours, 1, true, None)
        .expect("hourly schedule must validate");

    let now = utc_datetime("2026-03-12T10:00:00Z");
    let clock = SequenceClock::new([now]);
    let strategy = TimedFileRotationStrategy::new_with_clock(path, schedule.clone(), 1, clock);

    let expected = schedule.next_rollover(now);
    assert_eq!(
        strategy.next_rollover_at(),
        expected,
        "next_rollover_at must be seeded from clock.now()"
    );
}
