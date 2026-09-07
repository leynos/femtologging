//! Tests for timed rotation core logic.

use std::{
    fs::{self, OpenOptions},
    io::{BufWriter, Write},
    time::{Duration as StdDuration, SystemTime},
};

use chrono::{Duration, NaiveTime, Utc};
use filetime::FileTime;
use rstest::rstest;
use tempfile::tempdir;

use super::{
    core::TimedFileRotationStrategy,
    schedule::TimedRotationSchedule,
    test_helpers::utc_datetime,
};
use crate::{
    formatter::DefaultFormatter,
    handler::FemtoHandlerTrait,
    handlers::{
        file::{HandlerConfig, OverflowPolicy, RotationStrategy},
        timed_rotating::{
            TimedRotationConfig,
            clock::SequenceClock,
            core::FemtoTimedRotatingFileHandler,
            schedule::TimedRotationWhen,
        },
    },
    level::FemtoLevel,
    log_record::FemtoLogRecord,
};

#[derive(Debug)]
struct RotationPruningCase {
    backup_count: usize,
    create_notes_file: bool,
    expect_oldest_exists: bool,
    expect_recent_exists: bool,
    notes_assertion_msg: &'static str,
    oldest_assertion_msg: &'static str,
    recent_assertion_msg: &'static str,
}

#[rstest]
#[case::prunes_with_backup_count_1(RotationPruningCase { backup_count: 1, create_notes_file: true, expect_oldest_exists: false, expect_recent_exists: true, notes_assertion_msg: "non-rotated siblings must not be pruned as backups", oldest_assertion_msg: "oldest timed backup must be pruned", recent_assertion_msg: "most recent timed backup must remain" })]
#[case::retains_all_with_backup_count_0(RotationPruningCase { backup_count: 0, create_notes_file: false, expect_oldest_exists: true, expect_recent_exists: true, notes_assertion_msg: "", oldest_assertion_msg: "first timed backup must be retained when backup_count is zero", recent_assertion_msg: "second timed backup must be retained when backup_count is zero" })]
fn rotation_and_pruning_behaviour(#[case] case: RotationPruningCase) {
    let dir = tempdir().expect("tempdir must create a temporary directory");
    let path = dir.path().join("timed.log");
    let notes_path = dir.path().join("timed.log.notes");
    let schedule = TimedRotationSchedule::new(TimedRotationWhen::Seconds, 1, true, None)
        .expect("seconds schedule must validate");
    let start = utc_datetime!("2026-03-12T00:00:00Z");
    let clock = SequenceClock::new([
        start,
        start,
        start + Duration::seconds(2),
        start + Duration::seconds(4),
    ]);
    let mut strategy =
        TimedFileRotationStrategy::new_with_clock(path.clone(), schedule, case.backup_count, clock);
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

    if case.create_notes_file {
        fs::write(&notes_path, "keep me").expect("sibling file must be created");
    }

    RotationStrategy::before_write(&mut strategy, &mut writer, "third")
        .expect("second rotation must succeed");
    writer
        .write_all(b"third\n")
        .expect("third record must be written");
    writer.flush().expect("third flush must succeed");

    if case.create_notes_file {
        assert!(notes_path.exists(), "{}", case.notes_assertion_msg);
    }

    let oldest_path = path.with_file_name("timed.log.2026-03-12_00-00-00");
    assert_eq!(
        oldest_path.exists(),
        case.expect_oldest_exists,
        "{}",
        case.oldest_assertion_msg
    );

    let recent_path = path.with_file_name("timed.log.2026-03-12_00-00-02");
    assert_eq!(
        recent_path.exists(),
        case.expect_recent_exists,
        "{}",
        case.recent_assertion_msg
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

    let next = schedule.next_rollover(utc_datetime!("2026-03-11T23:59:59Z"));

    assert_eq!(next, utc_datetime!("2026-03-12T00:00:00Z"));
}

#[rstest]
fn seed_rollover_from_overrides_initial_rollover() {
    let dir = tempdir().expect("tempdir must create a temporary directory");
    let path = dir.path().join("timed.log");
    let schedule = TimedRotationSchedule::new(TimedRotationWhen::Hours, 1, true, None)
        .expect("hourly schedule must validate");

    let now = utc_datetime!("2026-03-12T10:00:00Z");
    let mtime = utc_datetime!("2026-03-12T08:00:00Z");
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

    let now = utc_datetime!("2026-03-12T10:00:00Z");
    let clock = SequenceClock::new([now]);
    let strategy = TimedFileRotationStrategy::new_with_clock(path, schedule.clone(), 1, clock);

    let expected = schedule.next_rollover(now);
    assert_eq!(
        strategy.next_rollover_at(),
        expected,
        "next_rollover_at must be seeded from clock.now()"
    );
}

#[rstest]
fn production_handler_seeds_rollover_from_file_mtime() {
    let dir = tempdir().expect("tempdir must create a temporary directory");
    let path = dir.path().join("timed.log");

    // Create the log file and set its mtime well in the past (relative to
    // the real clock the production handler uses) so the rollover deadline
    // seeded from that mtime (mtime + 1h) has already elapsed. If the
    // handler seeded `next_rollover_at` from `now` instead of the file's
    // mtime, the first write below would not trigger a rotation.
    fs::write(&path, "initial content\n").expect("log file must be created");
    let mtime_datetime = Utc::now() - Duration::hours(3);
    let mtime_secs = u64::try_from(mtime_datetime.timestamp())
        .expect("mtime timestamp must be after the Unix epoch");
    let mtime_systime = SystemTime::UNIX_EPOCH + StdDuration::from_secs(mtime_secs);
    let file_time = FileTime::from_system_time(mtime_systime);
    filetime::set_file_mtime(&path, file_time).expect("mtime must be set");

    // Build handler through production path
    let schedule = TimedRotationSchedule::new(TimedRotationWhen::Hours, 1, true, None)
        .expect("hourly schedule must validate");
    let config = HandlerConfig {
        capacity: 1024,
        flush_interval: 1,
        overflow_policy: OverflowPolicy::Block,
    };
    let rotation_config = TimedRotationConfig {
        schedule: schedule.clone(),
        backup_count: 1,
    };
    let handler = FemtoTimedRotatingFileHandler::with_capacity_flush_policy(
        &path,
        DefaultFormatter,
        config,
        rotation_config,
    )
    .expect("handler must be created");

    handler
        .handle(FemtoLogRecord::new("core", FemtoLevel::Info, "after seed"))
        .expect("record must be queued");
    // Dropping the handler waits for the worker thread to drain the queue,
    // flush, and shut down, so the rotation triggered by the first write is
    // guaranteed to be visible on disk afterwards.
    drop(handler);

    let expected_rollover = schedule.next_rollover(mtime_datetime);
    let backup_path = path.with_file_name(format!(
        "timed.log.{}",
        schedule.suffix_for(expected_rollover)
    ));
    assert!(
        backup_path.exists(),
        "rollover seeded from the file mtime should trigger an immediate rotation, producing \
         {backup_path:?}",
    );
    let backup_contents = fs::read_to_string(&backup_path).expect("backup file must be readable");
    assert_eq!(
        backup_contents, "initial content\n",
        "rotated backup must contain the pre-existing content",
    );

    let primary_contents = fs::read_to_string(&path).expect("primary log file must be readable");
    assert!(
        primary_contents.contains("after seed"),
        "primary log file must contain the new record after rotation: {primary_contents:?}",
    );
}
