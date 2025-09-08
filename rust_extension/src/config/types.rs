//! Type definitions and builder structs for femtologging configuration.

use std::convert::identity;
use std::{collections::BTreeMap, sync::Arc};

use pyo3::{prelude::*, Bound};
use thiserror::Error;

use crate::macros::{impl_as_pydict, py_setters, AsPyDict};

fn normalise_vec(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids.dedup();
    ids
}

use crate::{
    filters::{FilterBuildError, FilterBuilder},
    handler::FemtoHandlerTrait,
    handlers::{FileHandlerBuilder, HandlerBuildError, HandlerBuilderTrait, StreamHandlerBuilder},
    level::FemtoLevel,
};

/// Concrete handler builder variants.
#[derive(Clone, Debug)]
pub enum HandlerBuilder {
    /// Build a [`FemtoStreamHandler`].
    Stream(StreamHandlerBuilder),
    /// Build a [`FemtoFileHandler`].
    File(FileHandlerBuilder),
}

impl HandlerBuilder {
    pub(crate) fn build(&self) -> Result<Arc<dyn FemtoHandlerTrait>, HandlerBuildError> {
        match self {
            Self::Stream(b) => <StreamHandlerBuilder as HandlerBuilderTrait>::build_inner(b)
                .map(|h| Arc::new(h) as Arc<dyn FemtoHandlerTrait>),
            Self::File(b) => <FileHandlerBuilder as HandlerBuilderTrait>::build_inner(b)
                .map(|h| Arc::new(h) as Arc<dyn FemtoHandlerTrait>),
        }
    }
}

impl From<StreamHandlerBuilder> for HandlerBuilder {
    fn from(value: StreamHandlerBuilder) -> Self {
        Self::Stream(value)
    }
}

impl From<FileHandlerBuilder> for HandlerBuilder {
    fn from(value: FileHandlerBuilder) -> Self {
        Self::File(value)
    }
}

impl AsPyDict for HandlerBuilder {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<PyObject> {
        match self {
            Self::Stream(b) => b.as_pydict(py),
            Self::File(b) => b.as_pydict(py),
        }
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
    /// A logger referenced a filter identifier that was not defined.
    #[error("unknown filter id: {0}")]
    UnknownFilterId(String),
    /// Building a filter failed.
    #[error("failed to build filter {id}: {source}")]
    FilterBuild {
        /// The identifier of the filter that failed to build.
        id: String,
        /// The underlying build error.
        #[source]
        source: FilterBuildError,
    },
    /// Building a handler failed.
    #[error("failed to build handler {id}: {source}")]
    HandlerBuild {
        /// The identifier of the handler that failed to build.
        id: String,
        /// The underlying build error.
        #[source]
        source: HandlerBuildError,
    },
    /// Initialising a logger failed.
    #[error("failed to initialise logger: {0}")]
    LoggerInit(String),
}

/// Builder for formatter definitions.
#[pyclass]
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
}

/// Builder for logger configuration.
#[pyclass]
#[derive(Clone, Debug, Default)]
pub struct LoggerConfigBuilder {
    pub(crate) level: Option<FemtoLevel>,
    pub(crate) propagate: Option<bool>,
    pub(crate) filters: Vec<String>,
    pub(crate) handlers: Vec<String>,
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

    /// Set filters by identifier, replacing any existing filters.
    pub fn with_filters<I, S>(mut self, filter_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.filters = normalise_vec(filter_ids.into_iter().map(Into::into).collect());
        self
    }

    /// Set handlers by identifier, replacing any existing handlers.
    pub fn with_handlers<I, S>(mut self, handler_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.handlers = normalise_vec(handler_ids.into_iter().map(Into::into).collect());
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

    /// Retrieve the configured filter identifiers.
    pub fn filter_ids(&self) -> &[String] {
        &self.filters
    }

    /// Retrieve the configured handler identifiers.
    pub fn handler_ids(&self) -> &[String] {
        &self.handlers
    }
}

/// Builder for the overall configuration.
#[pyclass]
#[derive(Clone, Debug)]
pub struct ConfigBuilder {
    pub(crate) version: u8,
    pub(crate) disable_existing_loggers: bool,
    pub(crate) default_level: Option<FemtoLevel>,
    pub(crate) formatters: BTreeMap<String, FormatterBuilder>,
    pub(crate) filters: BTreeMap<String, FilterBuilder>,
    /// Registered handler builders keyed by identifier.
    ///
    /// `HandlerBuilder` is a concrete enum rather than a trait object to make
    /// cloning and serialisation straightforward.
    pub(crate) handlers: BTreeMap<String, HandlerBuilder>,
    pub(crate) loggers: BTreeMap<String, LoggerConfigBuilder>,
    pub(crate) root_logger: Option<LoggerConfigBuilder>,
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self {
            version: 1,
            disable_existing_loggers: false,
            default_level: None,
            formatters: BTreeMap::new(),
            filters: BTreeMap::new(),
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

    /// Add a formatter by identifier, replacing any existing formatter with the same id.
    pub fn with_formatter(mut self, id: impl Into<String>, builder: FormatterBuilder) -> Self {
        self.formatters.insert(id.into(), builder);
        self
    }

    /// Adds a filter configuration by its unique ID, replacing any existing entry.
    pub fn with_filter(mut self, id: impl Into<String>, builder: FilterBuilder) -> Self {
        self.filters.insert(id.into(), builder);
        self
    }

    /// Add a handler builder by identifier, replacing any existing handler with the same id.
    pub fn with_handler<B>(mut self, id: impl Into<String>, builder: B) -> Self
    where
        B: Into<HandlerBuilder>,
    {
        self.handlers.insert(id.into(), builder.into());
        self
    }

    /// Add a logger by name, replacing any existing logger with the same name.
    pub fn with_logger(mut self, name: impl Into<String>, builder: LoggerConfigBuilder) -> Self {
        self.loggers.insert(name.into(), builder);
        self
    }

    /// Set the root logger configuration, replacing any previous configuration.
    pub fn with_root_logger(mut self, builder: LoggerConfigBuilder) -> Self {
        self.root_logger = Some(builder);
        self
    }

    /// Return the configured version.
    pub fn version_get(&self) -> u8 {
        self.version
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

impl_as_pydict!(LoggerConfigBuilder {
    set_opt_to_string level => "level",
    set_opt propagate => "propagate",
    set_vec filters => "filters",
    set_vec handlers => "handlers",
});

py_setters!(LoggerConfigBuilder {
    level: py_with_level => "with_level", FemtoLevel, Some, "Set the logger level.",
    propagate: py_with_propagate => "with_propagate", bool, Some, "Set propagation behaviour.",
    filters: py_with_filters => "with_filters", Vec<String>, normalise_vec,
        "Set filters by identifier.\n\nThis replaces any existing filters with the provided list.",
    handlers: py_with_handlers => "with_handlers", Vec<String>, normalise_vec,
        "Set handlers by identifier.\n\nThis replaces any existing handlers with the provided list.",
});

impl_as_pydict!(ConfigBuilder {
    set_val version => "version",
    set_val disable_existing_loggers => "disable_existing_loggers",
    set_opt_to_string default_level => "default_level",
    set_map formatters => "formatters",
    set_map filters => "filters",
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

    /// Adds a filter configuration by its unique ID, replacing any existing entry.
    #[pyo3(name = "with_filter")]
    fn py_with_filter<'py>(
        mut slf: PyRefMut<'py, Self>,
        id: String,
        builder: Bound<'py, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let fb = builder.extract::<FilterBuilder>()?;
        slf.filters.insert(id, fb);
        Ok(slf)
    }

    /// Add a handler by identifier.
    #[pyo3(name = "with_handler")]
    fn py_with_handler<'py>(
        mut slf: PyRefMut<'py, Self>,
        id: String,
        builder: Bound<'py, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let hb = builder.extract::<HandlerBuilder>()?;
        slf.handlers.insert(id, hb);
        Ok(slf)
    }

    /// Finalise configuration, raising ``ValueError`` on error.
    #[pyo3(name = "build_and_init")]
    fn py_build_and_init(&self) -> PyResult<()> {
        self.build_and_init().map_err(PyErr::from)
    }
);
