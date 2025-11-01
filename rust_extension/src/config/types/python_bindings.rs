//! Python bindings for configuration builders.
#![cfg(feature = "python")]

use super::*;
use crate::macros::{impl_as_pydict, py_setters, AsPyDict};
use pyo3::{prelude::*, Bound};
use std::convert::identity;

impl AsPyDict for HandlerBuilder {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<PyObject> {
        match self {
            Self::Stream(b) => b.as_pydict(py),
            Self::File(b) => b.as_pydict(py),
            Self::Rotating(b) => b.as_pydict(py),
            Self::Socket(b) => b.as_pydict(py),
        }
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
    level: py_with_level => "with_level", FemtoLevel, Some, "Set the logger level, replacing any existing value.",
    propagate: py_with_propagate => "with_propagate", bool, Some, "Set propagation behaviour, replacing any existing value.",
    filters: py_with_filters => "with_filters", Vec<String>, normalise_vec,
        "Set filters by identifier.\n\nThis replaces any existing filters with the provided list.\nIDs are deduplicated and order may be normalised; see `normalise_vec`.",
    handlers: py_with_handlers => "with_handlers", Vec<String>, normalise_vec,
        "Set handlers by identifier.\n\nThis replaces any existing handlers with the provided list.\nIDs are deduplicated and order may be normalised; see `normalise_vec`.",
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
        "Set the schema version, replacing any existing value.",
    disable_existing_loggers: py_with_disable_existing_loggers =>
        "with_disable_existing_loggers", bool, identity,
        "Set whether existing loggers are disabled, replacing any existing value.",
    default_level: py_with_default_level => "with_default_level",
        FemtoLevel, Some, "Set the default log level, replacing any existing value.",
    root_logger: py_with_root_logger => "with_root_logger",
        LoggerConfigBuilder, Some,
        "Set the root logger configuration.\n\nCalling this multiple times replaces the previous root logger.",
};
    /// Add a formatter by identifier.
    ///
    /// Any existing formatter with the same identifier is replaced.
    #[pyo3(name = "with_formatter", text_signature = "(self, id, builder, /)")]
    fn py_with_formatter<'py>(
        mut slf: PyRefMut<'py, ConfigBuilder>,
        id: String,
        builder: FormatterBuilder,
    ) -> PyRefMut<'py, ConfigBuilder> {
        slf.formatters.insert(id, builder);
        slf
    }

    /// Add a logger by name.
    ///
    /// Any existing logger with the same name is replaced.
    #[pyo3(name = "with_logger", text_signature = "(self, name, builder, /)")]
    fn py_with_logger<'py>(
        mut slf: PyRefMut<'py, ConfigBuilder>,
        name: String,
        builder: LoggerConfigBuilder,
    ) -> PyRefMut<'py, ConfigBuilder> {
        slf.loggers.insert(name, builder);
        slf
    }

    /// Adds a filter configuration by its unique ID, replacing any existing entry.
    #[pyo3(name = "with_filter", text_signature = "(self, id, builder, /)")]
    fn py_with_filter<'py>(
        mut slf: PyRefMut<'py, ConfigBuilder>,
        id: String,
        builder: Bound<'py, PyAny>,
    ) -> PyResult<PyRefMut<'py, ConfigBuilder>> {
        let fb = builder.extract::<FilterBuilder>()?;
        slf.filters.insert(id, fb);
        Ok(slf)
    }

    /// Add a handler by identifier. Any existing handler with the same id is replaced.
    #[pyo3(name = "with_handler", text_signature = "(self, id, builder, /)")]
    fn py_with_handler<'py>(
        mut slf: PyRefMut<'py, ConfigBuilder>,
        id: String,
        builder: Bound<'py, PyAny>,
    ) -> PyResult<PyRefMut<'py, ConfigBuilder>> {
        let hb = builder.extract::<HandlerBuilder>()?;
        slf.handlers.insert(id, hb);
        Ok(slf)
    }

    /// Finalise configuration and initialise loggers.
    #[pyo3(name = "build_and_init", text_signature = "(self, /)")]
    fn py_build_and_init(&self) -> PyResult<()> {
        self.build_and_init().map_err(Into::into)
    }
);
