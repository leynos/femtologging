//! Timed rotation schedule calculations.
//!
//! The schedule stays pure so tests can validate rollover boundaries and
//! filename suffixes without touching the filesystem or sleeping.

use chrono::{
    DateTime, Datelike, Duration, Local, LocalResult, NaiveDateTime, NaiveTime, TimeZone, Utc,
    Weekday,
};

const MIDNIGHT: NaiveTime = NaiveTime::MIN;

/// Supported timed rotation cadences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimedRotationWhen {
    Seconds,
    Minutes,
    Hours,
    Days,
    Midnight,
    Weekday(Weekday),
}

impl TimedRotationWhen {
    /// Parse a stdlib-style ``when`` value.
    pub fn parse(value: &str) -> Result<Self, String> {
        let upper = value.to_ascii_uppercase();
        match upper.as_str() {
            "S" => Ok(Self::Seconds),
            "M" => Ok(Self::Minutes),
            "H" => Ok(Self::Hours),
            "D" => Ok(Self::Days),
            "MIDNIGHT" => Ok(Self::Midnight),
            weekday if weekday.len() == 2 && weekday.starts_with('W') => {
                let day = weekday[1..]
                    .parse::<u32>()
                    .map_err(|_| format!("unsupported timed rotation value: {value}"))?;
                let weekday = match day {
                    0 => Weekday::Mon,
                    1 => Weekday::Tue,
                    2 => Weekday::Wed,
                    3 => Weekday::Thu,
                    4 => Weekday::Fri,
                    5 => Weekday::Sat,
                    6 => Weekday::Sun,
                    _ => {
                        return Err(format!("unsupported timed rotation value: {value}",));
                    }
                };
                Ok(Self::Weekday(weekday))
            }
            _ => Err(format!("unsupported timed rotation value: {value}")),
        }
    }

    /// Return the canonical Python-facing ``when`` string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Seconds => "S",
            Self::Minutes => "M",
            Self::Hours => "H",
            Self::Days => "D",
            Self::Midnight => "MIDNIGHT",
            Self::Weekday(Weekday::Mon) => "W0",
            Self::Weekday(Weekday::Tue) => "W1",
            Self::Weekday(Weekday::Wed) => "W2",
            Self::Weekday(Weekday::Thu) => "W3",
            Self::Weekday(Weekday::Fri) => "W4",
            Self::Weekday(Weekday::Sat) => "W5",
            Self::Weekday(Weekday::Sun) => "W6",
        }
    }

    /// Return whether this cadence supports ``at_time``.
    pub const fn supports_at_time(self) -> bool {
        matches!(self, Self::Days | Self::Midnight | Self::Weekday(_))
    }
}

/// Validated timed rotation configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimedRotationSchedule {
    when: TimedRotationWhen,
    interval: u32,
    use_utc: bool,
    at_time: Option<NaiveTime>,
}

impl TimedRotationSchedule {
    /// Build a validated schedule.
    pub fn new(
        when: TimedRotationWhen,
        interval: u32,
        use_utc: bool,
        at_time: Option<NaiveTime>,
    ) -> Result<Self, String> {
        if interval == 0 {
            return Err("interval must be greater than zero".to_string());
        }
        if at_time.is_some() && !when.supports_at_time() {
            return Err(format!(
                "at_time is only supported for daily, midnight, and weekday rotation (got {})",
                when.as_str(),
            ));
        }
        Ok(Self {
            when,
            interval,
            use_utc,
            at_time,
        })
    }

    /// Return the configured cadence.
    #[cfg_attr(
        not(feature = "python"),
        expect(dead_code, reason = "python-only getter")
    )]
    pub const fn when(&self) -> TimedRotationWhen {
        self.when
    }

    /// Return the configured interval.
    #[cfg_attr(
        not(feature = "python"),
        expect(dead_code, reason = "python-only getter")
    )]
    pub const fn interval(&self) -> u32 {
        self.interval
    }

    /// Return whether UTC scheduling is enabled.
    #[cfg_attr(
        not(feature = "python"),
        expect(dead_code, reason = "python-only getter")
    )]
    pub const fn use_utc(&self) -> bool {
        self.use_utc
    }

    /// Return the optional time-of-day trigger.
    #[cfg_attr(
        not(feature = "python"),
        expect(dead_code, reason = "python-only getter")
    )]
    pub const fn at_time(&self) -> Option<NaiveTime> {
        self.at_time
    }

    /// Return the next rollover instant after ``now``.
    pub fn next_rollover(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        match self.when {
            TimedRotationWhen::Seconds => now + Duration::seconds(i64::from(self.interval)),
            TimedRotationWhen::Minutes => now + Duration::minutes(i64::from(self.interval)),
            TimedRotationWhen::Hours => now + Duration::hours(i64::from(self.interval)),
            TimedRotationWhen::Days if self.at_time.is_none() => {
                now + Duration::days(i64::from(self.interval))
            }
            TimedRotationWhen::Days => self.next_daily_rollover(now),
            TimedRotationWhen::Midnight => self.next_midnight_rollover(now),
            TimedRotationWhen::Weekday(weekday) => self.next_weekday_rollover(now, weekday),
        }
    }

    /// Return the suffix used for a file rolled over at ``rollover_at``.
    pub fn suffix_for(&self, rollover_at: DateTime<Utc>) -> String {
        let naive = self.local_naive(rollover_at);
        match self.when {
            TimedRotationWhen::Seconds => naive.format("%Y-%m-%d_%H-%M-%S").to_string(),
            TimedRotationWhen::Minutes => naive.format("%Y-%m-%d_%H-%M").to_string(),
            TimedRotationWhen::Hours => naive.format("%Y-%m-%d_%H").to_string(),
            TimedRotationWhen::Days
            | TimedRotationWhen::Midnight
            | TimedRotationWhen::Weekday(_) => naive.format("%Y-%m-%d").to_string(),
        }
    }

    fn next_rollover_at_trigger(
        &self,
        now: DateTime<Utc>,
        days_before_trigger: u32,
    ) -> DateTime<Utc> {
        let naive = self.local_naive(now);
        let trigger = self.at_time.unwrap_or(MIDNIGHT);
        let mut date = naive.date();
        let candidate = date.and_time(trigger);
        if naive >= candidate {
            date += Duration::days(i64::from(self.interval));
        } else {
            date += Duration::days(i64::from(days_before_trigger));
        }
        self.to_utc(date.and_time(trigger))
    }

    fn next_daily_rollover(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        self.next_rollover_at_trigger(now, 0)
    }

    fn next_midnight_rollover(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        self.next_rollover_at_trigger(now, self.interval.saturating_sub(1))
    }

    fn next_weekday_rollover(&self, now: DateTime<Utc>, weekday: Weekday) -> DateTime<Utc> {
        let naive = self.local_naive(now);
        let trigger = self.at_time.unwrap_or(MIDNIGHT);
        let date = naive.date();
        let mut days_ahead =
            weekday.num_days_from_monday() as i64 - date.weekday().num_days_from_monday() as i64;
        if days_ahead < 0 {
            days_ahead += 7;
        }
        if days_ahead == 0 && naive.time() >= trigger {
            days_ahead = 7;
        }
        days_ahead += i64::from(self.interval.saturating_sub(1)) * 7;
        self.to_utc((date + Duration::days(days_ahead)).and_time(trigger))
    }

    fn local_naive(&self, value: DateTime<Utc>) -> NaiveDateTime {
        if self.use_utc {
            value.naive_utc()
        } else {
            value.with_timezone(&Local).naive_local()
        }
    }

    fn to_utc(&self, value: NaiveDateTime) -> DateTime<Utc> {
        if self.use_utc {
            return Utc.from_utc_datetime(&value);
        }
        match Local.from_local_datetime(&value) {
            LocalResult::Single(dt) => dt.with_timezone(&Utc),
            LocalResult::Ambiguous(earliest, _) => earliest.with_timezone(&Utc),
            // DST gap: the requested local time doesn't exist (spring-forward).
            // Skip forward by small increments until we find a valid local time.
            LocalResult::None => {
                const MAX_DST_GAP_ATTEMPTS: u32 = 1_440;
                let mut candidate = value;
                let mut attempts = 0;
                while attempts < MAX_DST_GAP_ATTEMPTS {
                    attempts += 1;
                    candidate += Duration::minutes(1);
                    match Local.from_local_datetime(&candidate) {
                        LocalResult::Single(dt) => return dt.with_timezone(&Utc),
                        LocalResult::Ambiguous(earliest, _) => return earliest.with_timezone(&Utc),
                        LocalResult::None => continue,
                    }
                }
                // Fall back to interpreting the naive timestamp as UTC if the
                // local timezone never resolves within a full day.
                Utc.from_utc_datetime(&value)
            }
        }
    }
}
