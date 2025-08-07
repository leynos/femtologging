//! Builder types for configuring femtologging.
//!
//! These builders form the foundation of the configuration system. They
//! currently provide a minimal, type-safe API for defining formatters and
//! loggers. Handler builders will be added in a future iteration.

use std::collections::BTreeMap;

use pyo3::{exceptions::PyValueError, prelude::*, types::PyDict};
use thiserror::Error;

use crate::level::FemtoLevel;

/// Errors that may occur while building a configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The provided configuration schema version is unsupported.
    #[error("unsupported configuration version: {0}")]
    UnsupportedVersion(u8),
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

#[pymethods]
impl FormatterBuilder {
    #[new]
    fn py_new() -> Self {
        Self::new()
    }

    /// Set the format string.
    #[pyo3(name = "with_format")]
    fn py_with_format<'py>(mut slf: PyRefMut<'py, Self>, format: String) -> PyRefMut<'py, Self> {
        slf.format = Some(format);
        slf
    }

    /// Set the date format string.
    #[pyo3(name = "with_datefmt")]
    fn py_with_datefmt<'py>(mut slf: PyRefMut<'py, Self>, datefmt: String) -> PyRefMut<'py, Self> {
        slf.datefmt = Some(datefmt);
        slf
    }

    /// Return a dictionary representation of the formatter.
    fn as_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let d = PyDict::new(py);
        if let Some(ref f) = self.format {
            d.set_item("format", f)?;
        }
        if let Some(ref df) = self.datefmt {
            d.set_item("datefmt", df)?;
        }
        Ok(d.into())
    }
}

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
    pub fn with_filters(mut self, filter_ids: Vec<String>) -> Self {
        self.filters = filter_ids;
        self
    }

    /// Set handlers by identifier.
    pub fn with_handlers(mut self, handler_ids: Vec<String>) -> Self {
        self.handlers = handler_ids;
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

#[pymethods]
impl LoggerConfigBuilder {
    #[new]
    fn py_new() -> Self {
        Self::new()
    }

    /// Set the logger level.
    #[pyo3(name = "with_level")]
    fn py_with_level<'py>(mut slf: PyRefMut<'py, Self>, level: FemtoLevel) -> PyRefMut<'py, Self> {
        slf.level = Some(level);
        slf
    }

    /// Set propagation behaviour.
    #[pyo3(name = "with_propagate")]
    fn py_with_propagate<'py>(
        mut slf: PyRefMut<'py, Self>,
        propagate: bool,
    ) -> PyRefMut<'py, Self> {
        slf.propagate = Some(propagate);
        slf
    }

    /// Set filters by identifier.
    #[pyo3(name = "with_filters")]
    fn py_with_filters<'py>(mut slf: PyRefMut<'py, Self>, ids: Vec<String>) -> PyRefMut<'py, Self> {
        slf.filters = ids;
        slf
    }

    /// Set handlers by identifier.
    #[pyo3(name = "with_handlers")]
    fn py_with_handlers<'py>(
        mut slf: PyRefMut<'py, Self>,
        ids: Vec<String>,
    ) -> PyRefMut<'py, Self> {
        slf.handlers = ids;
        slf
    }

    /// Return a dictionary representation of the logger configuration.
    fn as_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let d = PyDict::new(py);
        if let Some(level) = self.level {
            d.set_item("level", level.to_string())?;
        }
        if let Some(propagate) = self.propagate {
            d.set_item("propagate", propagate)?;
        }
        if !self.filters.is_empty() {
            d.set_item("filters", &self.filters)?;
        }
        if !self.handlers.is_empty() {
            d.set_item("handlers", &self.handlers)?;
        }
        Ok(d.into())
    }
}

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
    pub fn with_formatter(mut self, id: impl Into<String>, builder: FormatterBuilder) -> Self {
        self.formatters.insert(id.into(), builder);
        self
    }

    /// Add a logger by name.
    pub fn with_logger(mut self, name: impl Into<String>, builder: LoggerConfigBuilder) -> Self {
        self.loggers.insert(name.into(), builder);
        self
    }

    /// Set the root logger configuration.
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
        Ok(())
    }

    fn create_formatters_dict(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        if self.formatters.is_empty() {
            return Ok(None);
        }
        let fmt = PyDict::new(py);
        for (k, v) in &self.formatters {
            fmt.set_item(k, v.as_dict(py)?)?;
        }
        Ok(Some(fmt.into()))
    }

    fn create_loggers_dict(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        if self.loggers.is_empty() {
            return Ok(None);
        }
        let lgs = PyDict::new(py);
        for (k, v) in &self.loggers {
            lgs.set_item(k, v.as_dict(py)?)?;
        }
        Ok(Some(lgs.into()))
    }

    /// Produce a Python dictionary describing the configuration.
    fn to_pydict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let d = PyDict::new(py);
        d.set_item("version", self.version)?;
        d.set_item("disable_existing_loggers", self.disable_existing_loggers)?;
        if let Some(level) = self.default_level {
            d.set_item("default_level", level.to_string())?;
        }
        if let Some(formatters) = self.create_formatters_dict(py)? {
            d.set_item("formatters", formatters)?;
        }
        if let Some(loggers) = self.create_loggers_dict(py)? {
            d.set_item("loggers", loggers)?;
        }
        if let Some(root) = &self.root_logger {
            d.set_item("root", root.as_dict(py)?)?;
        }
        Ok(d.into())
    }
}

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

    #[pyo3(name = "with_formatter")]
    fn py_with_formatter<'py>(
        mut slf: PyRefMut<'py, Self>,
        id: String,
        builder: FormatterBuilder,
    ) -> PyRefMut<'py, Self> {
        slf.formatters.insert(id, builder);
        slf
    }

    #[pyo3(name = "with_logger")]
    fn py_with_logger<'py>(
        mut slf: PyRefMut<'py, Self>,
        name: String,
        builder: LoggerConfigBuilder,
    ) -> PyRefMut<'py, Self> {
        slf.loggers.insert(name, builder);
        slf
    }

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
        self.to_pydict(py)
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
    fn build_accepts_default_version() {
        let builder = ConfigBuilder::new();
        assert!(builder.build_and_init().is_ok());
    }
}
