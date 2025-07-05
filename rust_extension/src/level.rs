use std::fmt;
use std::str::FromStr;

/// Severity levels for log records.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum FemtoLevel {
    Trace = 10,
    Debug = 20,
    Info = 30,
    Warn = 40,
    Error = 50,
    Critical = 60,
}

impl FemtoLevel {
    /// Return the string representation of the level.
    pub fn as_str(&self) -> &'static str {
        match self {
            FemtoLevel::Trace => "TRACE",
            FemtoLevel::Debug => "DEBUG",
            FemtoLevel::Info => "INFO",
            FemtoLevel::Warn => "WARN",
            FemtoLevel::Error => "ERROR",
            FemtoLevel::Critical => "CRITICAL",
        }
    }
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
            "TRACE" => Ok(FemtoLevel::Trace),
            "DEBUG" => Ok(FemtoLevel::Debug),
            "INFO" => Ok(FemtoLevel::Info),
            "WARN" | "WARNING" => Ok(FemtoLevel::Warn),
            "ERROR" => Ok(FemtoLevel::Error),
            "CRITICAL" | "FATAL" => Ok(FemtoLevel::Critical),
            _ => Err(()),
        }
    }
}
