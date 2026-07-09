//! Test helpers for timed rotation scheduling tests.
//!
//! These are macros rather than functions so the `expect` calls expand
//! inside the calling test, keeping panic line numbers at the call site and
//! satisfying the expect lint (which cannot see helper functions as tests).

#![cfg(test)]

/// Parse an RFC 3339 string into a UTC `DateTime`. Panics on invalid input.
macro_rules! utc_datetime {
    ($s:expr) => {
        $s.parse::<chrono::DateTime<chrono::Utc>>()
            .expect("test datetime must be valid")
    };
}

/// Build a `NaiveTime` from components. Panics on invalid components.
macro_rules! naive_time {
    ($hour:expr, $minute:expr, $second:expr) => {
        chrono::NaiveTime::from_hms_opt($hour, $minute, $second).expect("test time must be valid")
    };
}

pub(super) use {naive_time, utc_datetime};
