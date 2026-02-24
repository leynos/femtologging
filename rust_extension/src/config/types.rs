//! Type definitions and builder structs for femtologging configuration.

use std::{collections::BTreeMap, sync::Arc};

use thiserror::Error;

#[cfg(feature = "python")]
use pyo3::prelude::pyclass;

fn normalize_vec(ids: Vec<String>) -> Vec<String> {
    use std::collections::HashSet;
    let mut seen = HashSet::new();
    ids.into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect()
}

use crate::{
    filters::{FilterBuildError, FilterBuilder},
    handler::FemtoHandlerTrait,
    handlers::{
        FileHandlerBuilder, HandlerBuildError, HandlerBuilderTrait, RotatingFileHandlerBuilder,
        SocketHandlerBuilder, StreamHandlerBuilder,
    },
    level::FemtoLevel,
};

/// Concrete handler builder variants.
#[derive(Clone, Debug)]
pub enum HandlerBuilder {
    /// Build a [`FemtoStreamHandler`].
    Stream(StreamHandlerBuilder),
    /// Build a [`FemtoFileHandler`].
    File(FileHandlerBuilder),
    /// Build a [`FemtoRotatingFileHandler`].
    Rotating(RotatingFileHandlerBuilder),
    /// Build a [`FemtoSocketHandler`].
    Socket(SocketHandlerBuilder),
}

impl HandlerBuilder {
    #[cfg_attr(
        not(feature = "python"),
        expect(dead_code, reason = "unused without python feature")
    )]
    pub(crate) fn build(&self) -> Result<Arc<dyn FemtoHandlerTrait>, HandlerBuildError> {
        match self {
            Self::Stream(b) => <StreamHandlerBuilder as HandlerBuilderTrait>::build_inner(b)
                .map(|h| Arc::new(h) as Arc<dyn FemtoHandlerTrait>),
            Self::File(b) => <FileHandlerBuilder as HandlerBuilderTrait>::build_inner(b)
                .map(|h| Arc::new(h) as Arc<dyn FemtoHandlerTrait>),
            Self::Rotating(b) => {
                <RotatingFileHandlerBuilder as HandlerBuilderTrait>::build_inner(b)
                    .map(|h| Arc::new(h) as Arc<dyn FemtoHandlerTrait>)
            }
            Self::Socket(b) => <SocketHandlerBuilder as HandlerBuilderTrait>::build_inner(b)
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

impl From<RotatingFileHandlerBuilder> for HandlerBuilder {
    fn from(value: RotatingFileHandlerBuilder) -> Self {
        Self::Rotating(value)
    }
}

impl From<SocketHandlerBuilder> for HandlerBuilder {
    fn from(value: SocketHandlerBuilder) -> Self {
        Self::Socket(value)
    }
}

/// Errors that may occur while building a configuration.
#[derive(Debug, Error)]
#[cfg_attr(
    not(feature = "python"),
    expect(dead_code, reason = "unused without python feature")
)]
pub enum ConfigError {
    /// The provided configuration schema version is unsupported.
    #[error("unsupported configuration version: {0}")]
    UnsupportedVersion(u8),
    /// No root logger configuration was provided.
    #[error("missing root logger configuration")]
    MissingRootLogger,
    /// One or more handler or filter identifiers were referenced but not defined.
    #[error("unknown ids: {0:?}")]
    UnknownIds(Vec<String>),
    /// Duplicate handler identifiers were provided.
    #[error("duplicate handler ids: {0:?}")]
    DuplicateHandlerIds(Vec<String>),
    /// Duplicate filter identifiers were provided.
    #[error("duplicate filter ids: {0:?}")]
    DuplicateFilterIds(Vec<String>),
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
    /// Initializing a logger failed.
    #[error("failed to initialize logger: {0}")]
    LoggerInit(String),
}

/// Builder for formatter definitions.
#[cfg_attr(feature = "python", pyclass(from_py_object))]
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

/// Builder for logger configuration.
#[cfg_attr(feature = "python", pyclass(from_py_object))]
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

    /// Set the logger level, replacing any existing value.
    pub fn with_level(mut self, level: FemtoLevel) -> Self {
        self.level = Some(level);
        self
    }

    /// Set propagation behaviour, replacing any existing value.
    pub fn with_propagate(mut self, propagate: bool) -> Self {
        self.propagate = Some(propagate);
        self
    }

    /// Set filters by identifier, replacing any existing filters.
    /// IDs are deduplicated and order may be normalized; see [`normalize_vec`].
    pub fn with_filters<I, S>(mut self, filter_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.filters = normalize_vec(filter_ids.into_iter().map(Into::into).collect());
        self
    }

    /// Set handlers by identifier, replacing any existing handlers.
    /// IDs are deduplicated and order may be normalized; see [`normalize_vec`].
    pub fn with_handlers<I, S>(mut self, handler_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.handlers = normalize_vec(handler_ids.into_iter().map(Into::into).collect());
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
#[cfg_attr(feature = "python", pyclass(from_py_object))]
#[derive(Clone, Debug)]
pub struct ConfigBuilder {
    version: u8,
    disable_existing_loggers: bool,
    default_level: Option<FemtoLevel>,
    formatters: BTreeMap<String, FormatterBuilder>,
    filters: BTreeMap<String, FilterBuilder>,
    /// Registered handler builders keyed by identifier.
    ///
    /// `HandlerBuilder` is a concrete enum rather than a trait object to make
    /// cloning and serialization straightforward.
    handlers: BTreeMap<String, HandlerBuilder>,
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

    /// Set the schema version, replacing any existing value.
    pub fn with_version(mut self, version: u8) -> Self {
        self.version = version;
        self
    }

    /// Set whether existing loggers are disabled, replacing any existing value.
    pub fn with_disable_existing_loggers(mut self, disable: bool) -> Self {
        self.disable_existing_loggers = disable;
        self
    }

    /// Set the default log level, replacing any existing value.
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

    /// Determine whether existing loggers should be disabled.
    pub fn disable_existing_loggers(&self) -> bool {
        self.disable_existing_loggers
    }

    /// Retrieve the default log level if configured.
    pub fn default_level(&self) -> Option<FemtoLevel> {
        self.default_level
    }

    /// Retrieve configured handler builders.
    pub fn handler_builders(&self) -> &BTreeMap<String, HandlerBuilder> {
        &self.handlers
    }

    /// Retrieve configured filter builders.
    pub fn filter_builders(&self) -> &BTreeMap<String, FilterBuilder> {
        &self.filters
    }

    /// Retrieve configured logger builders.
    pub fn logger_builders(&self) -> &BTreeMap<String, LoggerConfigBuilder> {
        &self.loggers
    }

    /// Retrieve the root logger configuration if set.
    pub fn root_logger(&self) -> Option<&LoggerConfigBuilder> {
        self.root_logger.as_ref()
    }

    /// Return the configured version.
    pub fn version(&self) -> u8 {
        self.version
    }
}

#[cfg(feature = "python")]
mod python_bindings;
