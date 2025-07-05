//! Log severity levels used by [`FemtoLogger`].
//!
//! This module defines the [`FemtoLevel`] enum and helper functions for
//! converting between strings and numeric representations so loggers can
//! efficiently filter records.

use std::fmt;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum FemtoLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Critical,
}

impl Default for FemtoLevel {
    fn default() -> Self {
        Self::Info
    }
}

impl fmt::Display for FemtoLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            FemtoLevel::Trace => "TRACE",
            FemtoLevel::Debug => "DEBUG",
            FemtoLevel::Info => "INFO",
            FemtoLevel::Warn => "WARN",
            FemtoLevel::Error => "ERROR",
            FemtoLevel::Critical => "CRITICAL",
        };
        f.write_str(s)
    }
}

impl FromStr for FemtoLevel {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_uppercase().as_str() {
            "TRACE" => Ok(Self::Trace),
            "DEBUG" => Ok(Self::Debug),
            "INFO" => Ok(Self::Info),
            "WARN" | "WARNING" => Ok(Self::Warn),
            "ERROR" => Ok(Self::Error),
            "CRITICAL" => Ok(Self::Critical),
            _ => Err(()),
        }
    }
}

impl FemtoLevel {
    /// Parse a string into a level, warning on invalid input.
    pub fn parse_or_warn(s: &str) -> Self {
        match s.parse() {
            Ok(lvl) => lvl,
            Err(_) => {
                eprintln!("Warning: unrecognised log level '{s}', defaulting to INFO");
                Self::Info
            }
        }
    }
}

impl From<FemtoLevel> for u8 {
    fn from(level: FemtoLevel) -> Self {
        level as u8
    }
}

impl TryFrom<u8> for FemtoLevel {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, <Self as TryFrom<u8>>::Error> {
        match value {
            0 => Ok(Self::Trace),
            1 => Ok(Self::Debug),
            2 => Ok(Self::Info),
            3 => Ok(Self::Warn),
            4 => Ok(Self::Error),
            5 => Ok(Self::Critical),
            _ => Err(()),
        }
    }
}
