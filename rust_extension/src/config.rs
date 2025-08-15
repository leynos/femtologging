//! Builder types for configuring femtologging.
//!
//! These builders form the foundation of the configuration system. They
//! currently provide a minimal, type-safe API for defining formatters and
//! loggers. Handler builders will be added in a future iteration.

use std::{collections::BTreeMap, convert::identity, sync::Arc};

use pyo3::{exceptions::PyValueError, prelude::*};
use thiserror::Error;

use crate::{
    handler::FemtoHandlerTrait,
    handlers::{FileHandlerBuilder, HandlerBuildError, HandlerBuilderTrait, StreamHandlerBuilder},
    level::FemtoLevel,
    logger::FemtoLogger,
    macros::{impl_as_pydict, py_setters, AsPyDict},
    manager,
};

trait DynHandlerBuilderTrait: AsPyDict + Send + Sync + std::fmt::Debug {
    fn build_dyn(&self) -> Result<Arc<dyn FemtoHandlerTrait>, HandlerBuildError>;
}

impl<T> DynHandlerBuilderTrait for T
where
    T: HandlerBuilderTrait + AsPyDict + Send + Sync + std::fmt::Debug,
{
    fn build_dyn(&self) -> Result<Arc<dyn FemtoHandlerTrait>, HandlerBuildError> {
        T::build(self).map(Arc::from)
    }
}

type DynHandlerBuilder = Box<dyn DynHandlerBuilderTrait>;

impl AsPyDict for Box<dyn DynHandlerBuilderTrait> {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<PyObject> {
        (**self).as_pydict(py)
    }
}

/// Errors that may occur while building a configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The provided configuration schema version is unsupported.
    #[error("unsupported configuration version: {0}")]
    UnsupportedVersion(u8),
    /// No root logger configuration was provided.
    #[error("missing root logger configuration")]
    MissingRootLogger,
    /// A logger referenced a handler identifier that was not defined.
    #[error("unknown handler id: {0}")]
    UnknownHandlerId(String),
    /// Building a handler failed.
    #[error("failed to build handler {id}: {source}")]
    HandlerBuild {
        id: String,
        #[source]
        source: HandlerBuildError,
    },
    /// Initialising a logger failed.
    #[error("failed to initialise logger: {0}")]
    LoggerInit(String),
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

    /// Retrieve the configured handler identifiers.
    pub fn handler_ids(&self) -> &[String] {
        &self.handlers
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
#[derive(Debug)]
pub struct ConfigBuilder {
    version: u8,
    disable_existing_loggers: bool,
    default_level: Option<FemtoLevel>,
    formatters: BTreeMap<String, FormatterBuilder>,
    handlers: BTreeMap<String, DynHandlerBuilder>,
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
            handlers: BTreeMap::new(),
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

    /// Add a handler builder by identifier.
    ///
    /// Any existing handler with the same identifier is replaced.
    pub fn with_handler<B>(mut self, id: impl Into<String>, builder: B) -> Self
    where
        B: HandlerBuilderTrait + AsPyDict + Send + Sync + std::fmt::Debug + 'static,
    {
        self.handlers.insert(id.into(), Box::new(builder));
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
        let mut built_handlers: BTreeMap<String, Arc<dyn FemtoHandlerTrait>> = BTreeMap::new();
        for (id, builder) in &self.handlers {
            let handler = builder
                .build_dyn()
                .map_err(|source| ConfigError::HandlerBuild {
                    id: id.clone(),
                    source,
                })?;
            built_handlers.insert(id.clone(), handler);
        }

        Python::with_gil(|py| -> Result<(), ConfigError> {
            let mut targets: Vec<(&str, &LoggerConfigBuilder)> = Vec::new();
            if let Some(root_cfg) = &self.root_logger {
                targets.push(("root", root_cfg));
            }
            targets.extend(self.loggers.iter().map(|(n, c)| (n.as_str(), c)));

            for (name, cfg) in targets {
                let logger = self.fetch_logger(py, name)?;
                self.apply_logger_config(py, &logger, cfg, &built_handlers)?;
            }
            Ok(())
        })?;
        Ok(())
    }

    fn fetch_logger<'py>(
        &self,
        py: Python<'py>,
        name: &str,
    ) -> Result<Py<FemtoLogger>, ConfigError> {
        manager::get_logger(py, name).map_err(|e| ConfigError::LoggerInit(e.to_string()))
    }

    fn apply_logger_config<'py>(
        &self,
        py: Python<'py>,
        logger: &Py<FemtoLogger>,
        cfg: &LoggerConfigBuilder,
        handlers: &BTreeMap<String, Arc<dyn FemtoHandlerTrait>>,
    ) -> Result<(), ConfigError> {
        if let Some(level) = cfg.level_opt() {
            logger.borrow(py).set_level(level);
        }
        for hid in cfg.handler_ids() {
            let h = handlers
                .get(hid)
                .ok_or_else(|| ConfigError::UnknownHandlerId(hid.clone()))?;
            logger.borrow(py).add_handler(h.clone());
        }
        Ok(())
    }
}

impl_as_pydict!(ConfigBuilder {
    set_val version => "version",
    set_val disable_existing_loggers => "disable_existing_loggers",
    set_opt_to_string default_level => "default_level",
    set_map formatters => "formatters",
    set_map handlers => "handlers",
    set_map loggers => "loggers",
    set_optmap root_logger => "root",
});
py_setters!(ConfigBuilder {
    version: py_with_version => "with_version", u8, identity,
        "Set the schema version.",
    disable_existing_loggers: py_with_disable_existing_loggers =>
        "with_disable_existing_loggers", bool, identity,
        "Set whether existing loggers are disabled.",
    default_level: py_with_default_level => "with_default_level",
        FemtoLevel, Some, "Set the default log level.",
    root_logger: py_with_root_logger => "with_root_logger",
        LoggerConfigBuilder, Some,
        "Set the root logger configuration.\n\nCalling this multiple times replaces the previous root logger.",
};
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

    /// Add a handler by identifier.
    #[pyo3(name = "with_handler")]
    fn py_with_handler<'py>(
        mut slf: PyRefMut<'py, Self>,
        id: String,
        builder: Bound<'py, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        if let Ok(b) = builder.extract::<StreamHandlerBuilder>() {
            slf.handlers.insert(id, Box::new(b));
            return Ok(slf);
        }
        if let Ok(b) = builder.extract::<FileHandlerBuilder>() {
            slf.handlers.insert(id, Box::new(b));
            return Ok(slf);
        }
        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "builder must be StreamHandlerBuilder or FileHandlerBuilder",
        ))
    }

    /// Finalise configuration, raising ``ValueError`` on error.
    #[pyo3(name = "build_and_init")]
    fn py_build_and_init(&self) -> PyResult<()> {
        self.build_and_init().map_err(PyErr::from)
    }
);

#[cfg(test)]
mod tests {
    use super::*;
    use pyo3::Python;
    use rstest::rstest;
    // Serialise tests that mutate the global manager to avoid race conditions.
    use serial_test::serial;

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

    #[rstest]
    #[serial]
    fn shared_handler_attached_once() {
        Python::with_gil(|py| {
            manager::reset_manager();
            let handler = StreamHandlerBuilder::stderr();
            let logger_cfg = LoggerConfigBuilder::new().with_handlers(["h"]);
            let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
            let builder = ConfigBuilder::new()
                .with_handler("h", handler)
                .with_root_logger(root)
                .with_logger("first", logger_cfg.clone())
                .with_logger("second", logger_cfg);
            builder.build_and_init().expect("build should succeed");
            let first = manager::get_logger(py, "first").unwrap();
            let second = manager::get_logger(py, "second").unwrap();
            let h1 = first.borrow(py).handlers_for_test();
            let h2 = second.borrow(py).handlers_for_test();
            assert_eq!(h1[0], h2[0], "handler should be shared");
        });
    }

    #[rstest]
    #[serial]
    fn unknown_handler_id_rejected() {
        let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
        let logger_cfg = LoggerConfigBuilder::new().with_handlers(["missing"]);
        let builder = ConfigBuilder::new()
            .with_root_logger(root)
            .with_logger("child", logger_cfg);
        let err = builder.build_and_init().unwrap_err();
        assert!(matches!(err, ConfigError::UnknownHandlerId(id) if id == "missing"));
    }
}
