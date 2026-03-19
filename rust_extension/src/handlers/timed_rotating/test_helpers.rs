#![cfg(test)]

use chrono::{DateTime, NaiveTime, Utc};

/// Parse an RFC 3339 string into a UTC [`DateTime`]. Panics on invalid input.
pub(super) fn utc_datetime(s: &str) -> DateTime<Utc> {
    s.parse::<DateTime<Utc>>()
        .expect("test datetime must be valid")
}

/// Build a [`NaiveTime`] for test inputs. Panics on invalid components.
pub(super) fn naive_time(hour: u32, minute: u32, second: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(hour, minute, second).expect("test time must be valid")
}
