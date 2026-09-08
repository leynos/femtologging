//! Builder for percent-style formatter definitions.

#[cfg(feature = "python")]
use pyo3::prelude::pyclass;

#[cfg(feature = "python")]
use crate::formatter::{PercentFormatter, SharedFormatter};

/// Builder for formatter definitions.
#[cfg_attr(feature = "python", pyclass(from_py_object))]
#[derive(Clone, Debug, Default)]
pub struct FormatterBuilder {
    pub(crate) format: Option<String>,
    pub(crate) datefmt: Option<String>,
}

impl FormatterBuilder {
    /// Create a new `FormatterBuilder`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the format string.
    pub fn with_format(mut self, format: impl Into<String>) -> Self {
        self.format = Some(format.into());
        self
    }

    /// Set the date format string.
    pub fn with_datefmt(mut self, datefmt: impl Into<String>) -> Self {
        self.datefmt = Some(datefmt.into());
        self
    }

    /// Return the configured format string.
    pub fn format_string(&self) -> Option<&str> {
        self.format.as_deref()
    }

    /// Return the configured date format string.
    pub fn datefmt_string(&self) -> Option<&str> {
        self.datefmt.as_deref()
    }

    /// Build the configured percent-style formatter.
    #[cfg(feature = "python")]
    pub(crate) fn build(&self) -> SharedFormatter {
        SharedFormatter::new(PercentFormatter::new(
            self.format
                .clone()
                .unwrap_or_else(|| "%(message)s".to_owned()),
            self.datefmt.clone(),
        ))
    }
}
