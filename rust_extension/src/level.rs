//! Log severity levels used by [`FemtoLogger`].
//!
//! This module defines the [`FemtoLevel`] enum and helper functions for
//! converting between strings and numeric representations so loggers can
//! efficiently filter records.

use pyo3::Borrowed;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use std::fmt;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
#[repr(u8)]
pub enum FemtoLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
    Critical,
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
    /// Return the canonical string representation of the level.
    ///
    /// This is a `const fn` enabling compile-time evaluation and zero-cost
    /// access to level names. Use this instead of [`Display`]/[`to_string()`]
    /// when you need a static string slice without allocation overhead.
    ///
    /// [`Display`]: std::fmt::Display
    /// [`to_string()`]: std::string::ToString::to_string
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
            Self::Critical => "CRITICAL",
        }
    }

    /// Parse a string into a level, warning on invalid input.
    pub fn parse_or_warn(s: &str) -> Self {
        match s.parse() {
            Ok(lvl) => lvl,
            Err(_) => {
                eprintln!("Warning: unrecognized log level '{s}', defaulting to INFO");
                Self::Info
            }
        }
    }

    /// Parse a string into a level, returning PyValueError on invalid input.
    ///
    /// Use this in PyO3 bindings instead of `parse_or_warn` to propagate
    /// errors to Python rather than silently defaulting.
    pub fn parse_py(s: &str) -> PyResult<Self> {
        match s.parse() {
            Ok(level) => Ok(level),
            Err(_) => Err(PyErr::new::<PyValueError, _>(format!(
                "invalid log level: {s}"
            ))),
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

impl<'a, 'py> FromPyObject<'a, 'py> for FemtoLevel {
    type Error = PyErr;

    fn extract(obj: Borrowed<'a, 'py, PyAny>) -> PyResult<Self> {
        let s: &str = obj.extract()?;
        FemtoLevel::parse_py(s)
    }
}
