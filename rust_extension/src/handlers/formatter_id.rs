//! Identifier for registered formatters.
//!
//! Distinguishes between built-in formatter identifiers and arbitrary
//! user-provided IDs to avoid scattering string comparisons.

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FormatterId {
    /// The built-in default formatter.
    Default,
    /// A user-specified formatter identifier.
    Custom(String),
}

impl FormatterId {
    /// Return the identifier as a string slice.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Default => "default",
            Self::Custom(id) => id.as_str(),
        }
    }
}

impl From<String> for FormatterId {
    fn from(id: String) -> Self {
        match id.as_str() {
            "default" => Self::Default,
            _ => Self::Custom(id),
        }
    }
}

impl From<&str> for FormatterId {
    fn from(id: &str) -> Self {
        Self::from(id.to_owned())
    }
}
