//! Log severity levels used by [`FemtoLogger`].
//!
//! This module defines the [`FemtoLevel`] enum and helper functions for
//! converting between strings and numeric representations so loggers can
//! efficiently filter records.

use pyo3::Borrowed;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use std::fmt;
use std::io::Write;
use std::str::FromStr;

/// A log record's severity, ordered from trace through critical.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
#[repr(u8)]
pub enum FemtoLevel {
    /// Detailed diagnostic events.
    Trace,
    /// Developer-oriented diagnostic events.
    Debug,
    #[default]
    /// General operational events.
    Info,
    /// Events indicating a potentially harmful condition.
    Warn,
    /// Events indicating an operation failed.
    Error,
    /// Events indicating a severe service failure.
    Critical,
}

impl fmt::Display for FemtoLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
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
    #[must_use]
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
    #[must_use]
    pub fn parse_or_warn(s: &str) -> Self {
        s.parse().unwrap_or_else(|()| Self::warn_and_default(s))
    }

    fn warn_and_default(s: &str) -> Self {
        let mut stderr = std::io::stderr().lock();
        drop(writeln!(
            stderr,
            "Warning: unrecognized log level '{s}', defaulting to INFO"
        ));
        Self::Info
    }

    /// Parse a string into a level, returning `PyValueError` on invalid input.
    ///
    /// Use this in `PyO3` bindings instead of [`Self::parse_or_warn`] to propagate
    /// errors to Python rather than silently defaulting.
    ///
    /// # Errors
    ///
    /// Returns `PyValueError` when `s` does not name a supported log level.
    pub fn parse_py(s: &str) -> PyResult<Self> {
        match s.parse() {
            Ok(level) => Ok(level),
            Err(()) => Err(PyErr::new::<PyValueError, _>(format!(
                "invalid log level: {s}"
            ))),
        }
    }
}

impl From<FemtoLevel> for u8 {
    fn from(level: FemtoLevel) -> Self {
        level as Self
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
        Self::parse_py(s)
    }
}
