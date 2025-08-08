//! Builder types for configuring femtologging.
//!
//! These builders form the foundation of the configuration system. They
//! currently provide a minimal, type-safe API for defining formatters and
//! loggers. Handler builders will be added in a future iteration.

use std::{collections::BTreeMap, convert::identity};

use pyo3::{exceptions::PyValueError, prelude::*};
use thiserror::Error;

use crate::{
    level::FemtoLevel,
    macros::{impl_as_pydict, py_setters, AsPyDict},
};

/// Errors that may occur while building a configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The provided configuration schema version is unsupported.
    #[error("unsupported configuration version: {0}")]
    UnsupportedVersion(u8),
    /// No root logger configuration was provided.
    #[error("missing root logger configuration")]
    MissingRootLogger,
}

impl From<ConfigError> for PyErr {
    fn from(err: ConfigError) -> Self {
        PyValueError::new_err(err.to_string())
    }
}

/// Builder for formatter definitions.
#[pyclass]
#[derive(Clone, Debug, Default)]
pub struct FormatterBuilder {
    format: Option<String>,
    datefmt: Option<String>,
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
}

impl_as_pydict!(FormatterBuilder {
    set_opt format => "format",
    set_opt datefmt => "datefmt",
});

py_setters!(FormatterBuilder {
    format: py_with_format => "with_format", String, Some, "Set the format string.",
    datefmt: py_with_datefmt => "with_datefmt", String, Some, "Set the date format string.",
});

/// Builder for logger configuration.
#[pyclass]
#[derive(Clone, Debug, Default)]
pub struct LoggerConfigBuilder {
    level: Option<FemtoLevel>,
    propagate: Option<bool>,
    filters: Vec<String>,
    handlers: Vec<String>,
}

impl LoggerConfigBuilder {
    /// Create a new `LoggerConfigBuilder`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the logger level.
    pub fn with_level(mut self, level: FemtoLevel) -> Self {
        self.level = Some(level);
        self
    }

    /// Set propagation behaviour.
    pub fn with_propagate(mut self, propagate: bool) -> Self {
        self.propagate = Some(propagate);
        self
    }

    /// Set filters by identifier.
    ///
    /// This replaces any existing filters with the provided list.
    ///
    /// # Expected input
    ///
    /// Accepts any iterable of items convertible into a [`String`].
    /// This includes collections such as `Vec`, `&[&str]`, or similar.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// builder.with_filters(vec!["filter1", "filter2"]);
    /// builder.with_filters(&["filter1", "filter2"]);
    /// ```
    pub fn with_filters<I, S>(mut self, filter_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.filters = filter_ids.into_iter().map(Into::into).collect();
        self
    }

    /// Set handlers by identifier.
    ///
    /// This replaces any existing handlers with the provided list.
    ///
    /// # Expected input
    ///
    /// Accepts any iterable of items convertible into a [`String`].
    /// This includes collections such as `Vec`, `&[&str]`, or similar.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// builder.with_handlers(vec!["console", "file"]);
    /// builder.with_handlers(&["console", "file"]);
    /// ```
    pub fn with_handlers<I, S>(mut self, handler_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.handlers = handler_ids.into_iter().map(Into::into).collect();
        self
    }

    /// Retrieve the level if configured.
    pub fn level_opt(&self) -> Option<FemtoLevel> {
        self.level
    }

    /// Retrieve the propagate flag if configured.
    pub fn propagate_opt(&self) -> Option<bool> {
        self.propagate
    }
}

impl_as_pydict!(LoggerConfigBuilder {
    set_opt_to_string level => "level",
    set_opt propagate => "propagate",
    set_vec filters => "filters",
    set_vec handlers => "handlers",
});

py_setters!(LoggerConfigBuilder {
    level: py_with_level => "with_level", FemtoLevel, Some, "Set the logger level.",
    propagate: py_with_propagate => "with_propagate", bool, Some, "Set propagation behaviour.",
    filters: py_with_filters => "with_filters", Vec<String>, identity,
        "Set filters by identifier.\n\nThis replaces any existing filters with the provided list.",
    handlers: py_with_handlers => "with_handlers", Vec<String>, identity,
        "Set handlers by identifier.\n\nThis replaces any existing handlers with the provided list.",
});

/// Top-level builder coordinating loggers and formatters.
#[pyclass]
#[derive(Clone, Debug)]
pub struct ConfigBuilder {
    version: u8,
    disable_existing_loggers: bool,
    default_level: Option<FemtoLevel>,
    formatters: BTreeMap<String, FormatterBuilder>,
    loggers: BTreeMap<String, LoggerConfigBuilder>,
    root_logger: Option<LoggerConfigBuilder>,
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self {
            version: 1,
            disable_existing_loggers: false,
            default_level: None,
            formatters: BTreeMap::new(),
            loggers: BTreeMap::new(),
            root_logger: None,
        }
    }
}

impl ConfigBuilder {
    /// Create a new `ConfigBuilder`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the schema version.
    pub fn with_version(mut self, version: u8) -> Self {
        self.version = version;
        self
    }

    /// Set whether existing loggers are disabled.
    pub fn with_disable_existing_loggers(mut self, disable: bool) -> Self {
        self.disable_existing_loggers = disable;
        self
    }

    /// Set the default log level.
    pub fn with_default_level(mut self, level: FemtoLevel) -> Self {
        self.default_level = Some(level);
        self
    }

    /// Add a formatter by identifier.
    ///
    /// Any existing formatter with the same identifier is replaced.
    pub fn with_formatter(mut self, id: impl Into<String>, builder: FormatterBuilder) -> Self {
        self.formatters.insert(id.into(), builder);
        self
    }

    /// Add a logger by name.
    ///
    /// Any existing logger with the same name is replaced.
    pub fn with_logger(mut self, name: impl Into<String>, builder: LoggerConfigBuilder) -> Self {
        self.loggers.insert(name.into(), builder);
        self
    }

    /// Set the root logger configuration.
    ///
    /// Calling this multiple times replaces the previous root logger.
    pub fn with_root_logger(mut self, builder: LoggerConfigBuilder) -> Self {
        self.root_logger = Some(builder);
        self
    }

    /// Return the configured version.
    pub fn version_get(&self) -> u8 {
        self.version
    }

    /// Finalise the configuration.
    pub fn build_and_init(&self) -> Result<(), ConfigError> {
        if self.version != 1 {
            return Err(ConfigError::UnsupportedVersion(self.version));
        }
        if self.root_logger.is_none() {
            return Err(ConfigError::MissingRootLogger);
        }
        Ok(())
    }
}

impl_as_pydict!(ConfigBuilder {
    set_val version => "version",
    set_val disable_existing_loggers => "disable_existing_loggers",
    set_opt_to_string default_level => "default_level",
    set_map formatters => "formatters",
    set_map loggers => "loggers",
    set_optmap root_logger => "root",
});

#[pymethods]
impl ConfigBuilder {
    #[new]
    fn py_new() -> Self {
        Self::new()
    }

    #[pyo3(name = "with_version")]
    fn py_with_version<'py>(mut slf: PyRefMut<'py, Self>, version: u8) -> PyRefMut<'py, Self> {
        slf.version = version;
        slf
    }

    #[pyo3(name = "with_disable_existing_loggers")]
    fn py_with_disable_existing_loggers<'py>(
        mut slf: PyRefMut<'py, Self>,
        disable: bool,
    ) -> PyRefMut<'py, Self> {
        slf.disable_existing_loggers = disable;
        slf
    }

    #[pyo3(name = "with_default_level")]
    fn py_with_default_level<'py>(
        mut slf: PyRefMut<'py, Self>,
        level: FemtoLevel,
    ) -> PyRefMut<'py, Self> {
        slf.default_level = Some(level);
        slf
    }

    /// Add a formatter by identifier.
    ///
    /// Any existing formatter with the same identifier is replaced.
    #[pyo3(name = "with_formatter")]
    fn py_with_formatter<'py>(
        mut slf: PyRefMut<'py, Self>,
        id: String,
        builder: FormatterBuilder,
    ) -> PyRefMut<'py, Self> {
        slf.formatters.insert(id, builder);
        slf
    }

    /// Add a logger by name.
    ///
    /// Any existing logger with the same name is replaced.
    #[pyo3(name = "with_logger")]
    fn py_with_logger<'py>(
        mut slf: PyRefMut<'py, Self>,
        name: String,
        builder: LoggerConfigBuilder,
    ) -> PyRefMut<'py, Self> {
        slf.loggers.insert(name, builder);
        slf
    }

    /// Set the root logger configuration.
    ///
    /// Calling this multiple times replaces the previous root logger.
    #[pyo3(name = "with_root_logger")]
    fn py_with_root_logger<'py>(
        mut slf: PyRefMut<'py, Self>,
        builder: LoggerConfigBuilder,
    ) -> PyRefMut<'py, Self> {
        slf.root_logger = Some(builder);
        slf
    }

    /// Finalise configuration, raising ``ValueError`` on error.
    #[pyo3(name = "build_and_init")]
    fn py_build_and_init(&self) -> PyResult<()> {
        self.build_and_init().map_err(PyErr::from)
    }

    /// Return a dictionary representation of the configuration.
    fn as_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        self.as_pydict(py)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    fn build_rejects_invalid_version() {
        let builder = ConfigBuilder::new().with_version(2);
        assert!(builder.build_and_init().is_err());
    }

    #[rstest]
    fn build_rejects_missing_root() {
        let builder = ConfigBuilder::new();
        assert!(builder.build_and_init().is_err());
    }

    #[rstest]
    fn build_accepts_default_version() {
        let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
        let builder = ConfigBuilder::new().with_root_logger(root);
        assert!(builder.build_and_init().is_ok());
    }
}
