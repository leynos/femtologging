use std::fmt;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
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
    pub fn parse_or_info(s: &str) -> Self {
        s.parse().unwrap_or(Self::Info)
    }
}
