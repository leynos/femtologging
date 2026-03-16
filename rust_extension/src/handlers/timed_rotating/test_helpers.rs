#![cfg(test)]

use chrono::{DateTime, Utc};

/// Parse an RFC 3339 string into a UTC [`DateTime`]. Panics on invalid input.
pub(super) fn utc_datetime(s: &str) -> DateTime<Utc> {
    s.parse::<DateTime<Utc>>()
        .expect("test datetime must be valid")
}
