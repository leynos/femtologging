//! Builder for [`FemtoStreamHandler`].
//!
//! Allows configuration of stream based handlers writing to `stdout` or
//! `stderr`. The builder exposes basic tuning for channel capacity and
//! a millisecond-based flush threshold. `py_new` defaults to `stderr`
//! to mirror Python's `logging.StreamHandler`.

use std::{
    io::{self, Write},
    num::NonZeroU64,
    time::Duration,
};

#[cfg(feature = "python")]
use pyo3::prelude::*;

use super::{
    FormatterId, HandlerBuildError, HandlerBuilderTrait,
    common::{CommonBuilder, FormatterConfig, IntoFormatterConfig},
};

use crate::handlers::builder_macros::builder_methods;
#[cfg(test)]
use crate::level::FemtoLevel;
#[cfg(feature = "python")]
use crate::macros::{AsPyDict, dict_into_py};
use crate::{
    formatter::{DefaultFormatter, FemtoFormatter},
    stream_handler::FemtoStreamHandler,
};

mod target;
use target::StreamTarget;

/// Builder for constructing [`FemtoStreamHandler`] instances.
#[cfg_attr(feature = "python", pyclass(from_py_object))]
#[derive(Clone, Debug)]
pub struct StreamHandlerBuilder {
    /// Destination stream selected by the builder's constructor.
    target: StreamTarget,
    /// Shared capacity, formatter, and flush-threshold options.
    common: CommonBuilder,
}

impl StreamHandlerBuilder {
    /// Create a builder targeting `stdout`.
    pub fn stdout() -> Self {
        Self {
            target: StreamTarget::Stdout,
            common: CommonBuilder::default(),
        }
    }

    /// Create a builder targeting `stderr`.
    pub fn stderr() -> Self {
        Self {
            target: StreamTarget::Stderr,
            common: CommonBuilder::default(),
        }
    }

    /// Attach a formatter instance or identifier.
    pub fn with_formatter<F>(mut self, formatter: F) -> Self
    where
        F: IntoFormatterConfig,
    {
        self.common.set_formatter(formatter);
        self
    }

    /// Attach filters by identifier.
    pub fn with_filters<I, S>(mut self, filter_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.common.set_filter_ids(filter_ids);
        self
    }

    /// Replace a custom formatter identifier with its configured instance.
    #[cfg(feature = "python")]
    pub(crate) fn resolve_formatter(
        &mut self,
        formatters: &std::collections::BTreeMap<String, crate::formatter::SharedFormatter>,
    ) -> Result<(), HandlerBuildError> {
        self.common.resolve_formatter(formatters)
    }

    /// Return the configured handler filter identifiers.
    #[cfg(feature = "python")]
    pub(crate) fn filter_ids(&self) -> &[String] {
        self.common.filter_ids()
    }

    /// Rejects a zero flush threshold before constructing the worker.
    fn is_flush_after_ms_valid(&self) -> Result<(), HandlerBuildError> {
        CommonBuilder::ensure_non_zero(
            "flush_after_ms",
            self.common.flush_after_ms.map(NonZeroU64::get),
        )
    }

    /// Resolves the configured capacity, defaulting to the stream queue size.
    fn resolved_capacity(&self) -> usize {
        self.common.capacity.map(|c| c.get()).unwrap_or(1024)
    }

    /// Resolves the non-zero flush threshold used by the worker.
    fn resolved_flush_after(&self) -> Duration {
        Duration::from_millis(
            self.common
                .flush_after_ms
                .map(NonZeroU64::get)
                .unwrap_or(CommonBuilder::DEFAULT_FLUSH_AFTER_MS),
        )
    }

    /// Selects stdout or stderr while retaining the caller's formatter.
    fn build_with_formatter<F>(&self, formatter: F) -> FemtoStreamHandler
    where
        F: FemtoFormatter + Send + 'static,
    {
        match self.target {
            StreamTarget::Stdout => self.build_with_writer(io::stdout(), formatter),
            StreamTarget::Stderr => self.build_with_writer(io::stderr(), formatter),
        }
    }

    /// Moves the writer and formatter into a worker with the resolved queue settings.
    fn build_with_writer<W, F>(&self, writer: W, formatter: F) -> FemtoStreamHandler
    where
        W: Write + Send + 'static,
        F: FemtoFormatter + Send + 'static,
    {
        let capacity = self.resolved_capacity();
        let flush_after = self.resolved_flush_after();
        FemtoStreamHandler::with_capacity_timeout(writer, formatter, capacity, flush_after)
    }

    /// Runs all builder validation before worker state is published.
    fn validate(&self) -> Result<(), HandlerBuildError> {
        self.common.validate()?;
        self.is_flush_after_ms_valid()?;
        Ok(())
    }
}

builder_methods! {
    impl StreamHandlerBuilder {
        capacity {
            self_ident = builder,
            setter = |builder_ref, capacity| {
                builder_ref.common.set_capacity(capacity);
            }
        };
        methods {
            method {
                doc: "Set the flush threshold in milliseconds.\n\n# Validation\n\nAccepts a `NonZeroU64` so both Rust and Python callers must provide a value greater than zero.",
                rust_name: with_flush_after_ms,
                py_fn: py_with_flush_after_ms,
                py_name: "with_flush_after_ms",
                py_text_signature: "(self, flush_ms)",
                rust_args: (flush_ms: NonZeroU64),
                py_args: (flush_ms: u64),
                py_prelude: {
                    let flush_ms = NonZeroU64::new(flush_ms).ok_or_else(|| {
                        pyo3::exceptions::PyValueError::new_err(
                            "flush_after_ms must be greater than zero",
                        )
                    })?;
                },
                self_ident: builder,
                body: {
                    builder.common.flush_after_ms = Some(flush_ms);
                }
            }
        }
        extra_py_methods {
            /// Create a new `StreamHandlerBuilder` defaulting to `stderr`.
            ///
            /// Mirrors Python's `logging.StreamHandler` default stream.
            #[new]
            fn py_new() -> Self {
                Self::stderr()
            }

            /// Constructs a Python builder targeting stdout.
            #[staticmethod]
            #[pyo3(name = "stdout")]
            fn py_stdout() -> Self {
                Self::stdout()
            }

            /// Constructs a Python builder targeting stderr.
            #[staticmethod]
            #[pyo3(name = "stderr")]
            fn py_stderr() -> Self {
                Self::stderr()
            }

            /// Applies a Python formatter to the builder, returning conversion errors.
            #[pyo3(name = "with_formatter")]
            #[pyo3(signature = (formatter))]
            #[pyo3(text_signature = "(self, formatter)")]
            fn py_with_formatter<'py>(
                mut slf: PyRefMut<'py, Self>,
                formatter: Bound<'py, PyAny>,
            ) -> PyResult<PyRefMut<'py, Self>> {
                slf.common.set_formatter_from_py(&formatter)?;
                Ok(slf)
            }

            /// Store filter IDs for resolution by the configuration builder.
            #[pyo3(name = "with_filters")]
            fn py_with_filters<'py>(
                mut slf: PyRefMut<'py, Self>,
                filter_ids: Vec<String>,
            ) -> PyRefMut<'py, Self> {
                slf.common.set_filter_ids(filter_ids);
                slf
            }

            /// Return a dictionary describing the builder configuration.
            fn as_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
                self.as_pydict(py)
            }

            /// Build the handler, raising ``HandlerConfigError`` or ``HandlerIOError`` on
            /// failure.
            fn build(&self) -> PyResult<FemtoStreamHandler> {
                <Self as HandlerBuilderTrait>::build_inner(self).map_err(PyErr::from)
            }
        }
    }
}

#[cfg(feature = "python")]
impl AsPyDict for StreamHandlerBuilder {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        use pyo3::types::PyDict;
        let d = PyDict::new(py);
        d.set_item("target", self.target.as_str())?;
        self.common.extend_py_dict(&d)?;
        dict_into_py(d, py)
    }
}

impl HandlerBuilderTrait for StreamHandlerBuilder {
    type Handler = FemtoStreamHandler;

    fn build_inner(&self) -> Result<Self::Handler, HandlerBuildError> {
        self.validate()?;
        let handler = match self.common.formatter.as_ref() {
            Some(FormatterConfig::Instance(fmt)) => self.build_with_formatter(fmt.clone_arc()),
            Some(FormatterConfig::Id(FormatterId::Default)) | None => {
                self.build_with_formatter(DefaultFormatter)
            }
            Some(FormatterConfig::Id(FormatterId::Custom(other))) => {
                return Err(HandlerBuildError::InvalidConfig(format!(
                    "unknown formatter id: {other}",
                )));
            }
        };
        Ok(handler)
    }
}

#[cfg(test)]
#[path = "stream_builder/tests.rs"]
mod tests;
