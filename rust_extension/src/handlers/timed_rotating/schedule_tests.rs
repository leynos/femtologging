//! Tests for timed rotation schedule logic.

use chrono::{NaiveTime, Weekday};
use rstest::rstest;

use super::schedule::{TimedRotationSchedule, TimedRotationWhen};
use super::test_helpers::{naive_time, utc_datetime};

#[rstest]
#[case("S", TimedRotationWhen::Seconds)]
#[case("m", TimedRotationWhen::Minutes)]
#[case("H", TimedRotationWhen::Hours)]
#[case("D", TimedRotationWhen::Days)]
#[case("MIDNIGHT", TimedRotationWhen::Midnight)]
#[case("w4", TimedRotationWhen::Weekday(Weekday::Fri))]
fn parse_supported_when_values(#[case] raw: &str, #[case] expected: TimedRotationWhen) {
    let actual = TimedRotationWhen::parse(raw).expect("when must parse");
    assert_eq!(actual, expected);
}

#[rstest]
fn reject_unknown_when_value() {
    let err = TimedRotationWhen::parse("fortnight").expect_err("unsupported value must fail");
    assert_eq!(err, "unsupported timed rotation value: fortnight");
}

#[rstest]
fn reject_zero_interval() {
    let err = TimedRotationSchedule::new(TimedRotationWhen::Hours, 0, true, None)
        .expect_err("zero interval must fail");
    assert_eq!(err, "interval must be greater than zero");
}

#[rstest]
fn reject_at_time_for_hourly_rotation() {
    let err = TimedRotationSchedule::new(
        TimedRotationWhen::Hours,
        1,
        true,
        Some(naive_time(3, 15, 0)),
    )
    .expect_err("hourly rotation must reject at_time");
    assert_eq!(
        err,
        "at_time is only supported for daily, midnight, and weekday rotation (got H)",
    );
}

#[rstest]
fn reject_weekday_interval_other_than_one() {
    let err = TimedRotationSchedule::new(TimedRotationWhen::Weekday(Weekday::Fri), 2, true, None)
        .expect_err("weekday rotation must reject multi-week intervals");

    assert_eq!(err, "weekday rotation only supports interval = 1");
}

#[rstest]
fn next_hourly_rollover_uses_fixed_duration() {
    let schedule = TimedRotationSchedule::new(TimedRotationWhen::Hours, 2, true, None)
        .expect("hourly schedule must validate");
    let now = utc_datetime("2026-03-11T08:30:00Z");

    let next = schedule.next_rollover(now);

    assert_eq!(next, utc_datetime("2026-03-11T10:30:00Z"));
}

#[rstest]
fn next_midnight_rollover_uses_start_of_day() {
    let schedule = TimedRotationSchedule::new(TimedRotationWhen::Midnight, 1, true, None)
        .expect("midnight schedule must validate");
    let now = utc_datetime("2026-03-11T23:30:00Z");

    let next = schedule.next_rollover(now);

    assert_eq!(next, utc_datetime("2026-03-12T00:00:00Z"));
}

#[rstest]
#[case(
    TimedRotationWhen::Days,
    naive_time(9, 30, 0),
    "2026-03-11T08:00:00Z",
    "2026-03-11T09:30:00Z"
)]
#[case(
    TimedRotationWhen::Weekday(Weekday::Fri),
    naive_time(6, 0, 0),
    "2026-03-11T12:00:00Z",
    "2026-03-13T06:00:00Z"
)]
fn next_rollover_with_at_time(
    #[case] when: TimedRotationWhen,
    #[case] at_time: NaiveTime,
    #[case] now: &str,
    #[case] expected: &str,
) {
    let schedule = TimedRotationSchedule::new(when, 1, true, Some(at_time))
        .expect("schedule with at_time must validate");
    assert_eq!(
        schedule.next_rollover(utc_datetime(now)),
        utc_datetime(expected),
    );
}

#[rstest]
fn next_hourly_rollover_local_time() {
    let schedule = TimedRotationSchedule::new(TimedRotationWhen::Hours, 1, false, None)
        .expect("hourly local schedule must validate");
    let now = utc_datetime("2026-03-11T08:00:00Z");

    let next = schedule.next_rollover(now);

    // Local-time path should still advance by one hour relative to the
    // input, although the exact UTC result depends on the host timezone.
    assert!(next > now, "local-time rollover must be in the future");
}

#[rstest]
fn midnight_with_explicit_at_time() {
    let at_time = naive_time(2, 30, 0);
    let schedule = TimedRotationSchedule::new(TimedRotationWhen::Midnight, 1, true, Some(at_time))
        .expect("midnight schedule with at_time must validate");
    let now = utc_datetime("2026-03-11T01:00:00Z");

    let next = schedule.next_rollover(now);

    assert_eq!(next, utc_datetime("2026-03-11T02:30:00Z"));
}

#[rstest]
fn midnight_suffix_is_date_only() {
    let schedule = TimedRotationSchedule::new(TimedRotationWhen::Midnight, 1, true, None)
        .expect("midnight schedule must validate");
    let rollover_at = utc_datetime("2026-03-12T00:00:00Z");

    let suffix = schedule.suffix_for(rollover_at);

    assert_eq!(suffix, "2026-03-11");
}

#[rstest]
fn second_suffix_includes_full_timestamp() {
    let schedule = TimedRotationSchedule::new(TimedRotationWhen::Seconds, 1, true, None)
        .expect("seconds schedule must validate");
    let rollover_at = utc_datetime("2026-03-12T07:08:09Z");

    let suffix = schedule.suffix_for(rollover_at);

    assert_eq!(suffix, "2026-03-12_07-08-08");
}
