//! Builder for [`FemtoStreamHandler`].
//!
//! Allows configuration of stream based handlers writing to `stdout` or
//! `stderr`. The builder exposes basic tuning for channel capacity and
//! a millisecond-based flush timeout. `py_new` defaults to `stderr`
//! to mirror Python's `logging.StreamHandler`.

use std::{num::NonZeroU64, sync::Arc, time::Duration};

#[cfg(feature = "python")]
use pyo3::prelude::*;

use super::{
    common::{CommonBuilder, FormatterConfig, IntoFormatterConfig},
    FormatterId, HandlerBuildError, HandlerBuilderTrait,
};

use crate::handlers::builder_macros::builder_methods;
#[cfg(feature = "python")]
use crate::macros::{dict_into_py, AsPyDict};
use crate::{formatter::DefaultFormatter, stream_handler::FemtoStreamHandler};

#[derive(Clone, Copy, Debug)]
enum StreamTarget {
    Stdout,
    Stderr,
}

impl StreamTarget {
    #[cfg(feature = "python")]
    fn as_str(&self) -> &'static str {
        match self {
            StreamTarget::Stdout => "stdout",
            StreamTarget::Stderr => "stderr",
        }
    }
}

/// Builder for constructing [`FemtoStreamHandler`] instances.
#[cfg_attr(feature = "python", pyclass)]
#[derive(Clone, Debug)]
pub struct StreamHandlerBuilder {
    target: StreamTarget,
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

    fn is_capacity_valid(&self) -> Result<(), HandlerBuildError> {
        self.common.is_capacity_valid()
    }

    fn is_flush_timeout_valid(&self) -> Result<(), HandlerBuildError> {
        CommonBuilder::ensure_non_zero(
            "flush_timeout_ms",
            self.common.flush_timeout_ms.map(NonZeroU64::get),
        )
    }

    fn validate(&self) -> Result<(), HandlerBuildError> {
        self.is_capacity_valid()?;
        self.is_flush_timeout_valid()?;
        Ok(())
    }

    #[cfg(feature = "python")]
    fn set_formatter_from_py(&mut self, formatter: &Bound<'_, PyAny>) -> PyResult<()> {
        if let Ok(fid) = formatter.extract::<String>() {
            self.common.set_formatter(fid);
            return Ok(());
        }
        let instance = crate::formatter::python::formatter_from_py(formatter)?;
        self.common.set_formatter(instance);
        Ok(())
    }
}

builder_methods! {
    impl StreamHandlerBuilder {
        capacity {
            self_ident = builder,
            setter = |builder, capacity| {
                builder.common.set_capacity(capacity);
            }
        };
        methods {
            method {
                doc: "Set the flush timeout in milliseconds.\n\n# Validation\n\nAccepts a `NonZeroU64` so both Rust and Python callers must provide a timeout greater than zero.",
                rust_name: with_flush_timeout_ms,
                py_fn: py_with_flush_timeout_ms,
                py_name: "with_flush_timeout_ms",
                py_text_signature: "(self, timeout_ms)",
                rust_args: (timeout_ms: NonZeroU64),
                py_args: (timeout_ms: u64),
                py_prelude: {
                    let timeout_ms = NonZeroU64::new(timeout_ms).ok_or_else(|| {
                        pyo3::exceptions::PyValueError::new_err(
                            "flush_timeout_ms must be greater than zero",
                        )
                    })?;
                },
                self_ident: builder,
                body: {
                    builder.common.flush_timeout_ms = Some(timeout_ms);
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

            #[staticmethod]
            #[pyo3(name = "stdout")]
            fn py_stdout() -> Self {
                Self::stdout()
            }

            #[staticmethod]
            #[pyo3(name = "stderr")]
            fn py_stderr() -> Self {
                Self::stderr()
            }

            #[pyo3(name = "with_formatter")]
            #[pyo3(signature = (formatter))]
            #[pyo3(text_signature = "(self, formatter)")]
            fn py_with_formatter<'py>(
                mut slf: PyRefMut<'py, Self>,
                formatter: Bound<'py, PyAny>,
            ) -> PyResult<PyRefMut<'py, Self>> {
                slf.set_formatter_from_py(&formatter)?;
                Ok(slf)
            }

            /// Return a dictionary describing the builder configuration.
            fn as_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
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
    fn as_pydict(&self, py: Python<'_>) -> PyResult<PyObject> {
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
        let capacity = self.common.capacity.map(|c| c.get()).unwrap_or(1024);
        let timeout = Duration::from_millis(
            self.common
                .flush_timeout_ms
                .map(NonZeroU64::get)
                .unwrap_or(1000),
        );
        let handler = match self.common.formatter.as_ref() {
            Some(FormatterConfig::Instance(fmt)) => match self.target {
                StreamTarget::Stdout => FemtoStreamHandler::with_capacity_timeout(
                    std::io::stdout(),
                    Arc::clone(fmt),
                    capacity,
                    timeout,
                ),
                StreamTarget::Stderr => FemtoStreamHandler::with_capacity_timeout(
                    std::io::stderr(),
                    Arc::clone(fmt),
                    capacity,
                    timeout,
                ),
            },
            Some(FormatterConfig::Id(FormatterId::Default)) | None => match self.target {
                StreamTarget::Stdout => FemtoStreamHandler::with_capacity_timeout(
                    std::io::stdout(),
                    DefaultFormatter,
                    capacity,
                    timeout,
                ),
                StreamTarget::Stderr => FemtoStreamHandler::with_capacity_timeout(
                    std::io::stderr(),
                    DefaultFormatter,
                    capacity,
                    timeout,
                ),
            },
            Some(FormatterConfig::Id(FormatterId::Custom(other))) => {
                return Err(HandlerBuildError::InvalidConfig(format!(
                    "unknown formatter id: {other}",
                )))
            }
        };
        Ok(handler)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::assert_build_err;
    use super::*;
    #[cfg(feature = "python")]
    use pyo3::Python;
    use rstest::rstest;

    use crate::{formatter::FemtoFormatter, log_record::FemtoLogRecord};

    #[derive(Clone, Copy, Debug)]
    struct UpperFormatter;

    impl FemtoFormatter for UpperFormatter {
        fn format(&self, record: &FemtoLogRecord) -> String {
            record.message.to_uppercase()
        }
    }

    #[rstest]
    #[case(StreamHandlerBuilder::stdout())]
    #[case(StreamHandlerBuilder::stderr())]
    fn build_stream_handler_with_capacity(#[case] builder: StreamHandlerBuilder) {
        let builder = builder.with_capacity(8);
        let mut handler = builder
            .build_inner()
            .expect("build_inner must succeed for a valid builder");
        handler.flush();
        handler.close();
    }

    #[rstest]
    #[case(StreamHandlerBuilder::stdout())]
    #[case(StreamHandlerBuilder::stderr())]
    fn build_stream_handler_with_custom_formatter(#[case] builder: StreamHandlerBuilder) {
        let builder = builder.with_formatter(UpperFormatter).with_capacity(4);
        let mut handler = builder
            .build_inner()
            .expect("build_inner must accept formatter instances");
        handler.flush();
        handler.close();
    }

    #[rstest]
    #[case(StreamHandlerBuilder::stdout())]
    #[case(StreamHandlerBuilder::stderr())]
    fn reject_zero_capacity(#[case] builder: StreamHandlerBuilder) {
        let builder = builder.with_capacity(0);
        assert_build_err(&builder, "build_inner must fail for zero capacity");
    }

    #[cfg(feature = "python")]
    #[test]
    fn python_rejects_zero_flush_timeout() {
        Python::with_gil(|py| {
            let builder = pyo3::Py::new(py, StreamHandlerBuilder::stderr())
                .expect("Py::new must create a stream builder");
            let err = builder
                .as_ref(py)
                .call_method1("with_flush_timeout_ms", (0,))
                .expect_err("with_flush_timeout_ms must reject zero");
            assert!(err.is_instance_of::<pyo3::exceptions::PyValueError>(py));
        });
    }

    #[rstest]
    #[case(StreamHandlerBuilder::stdout())]
    #[case(StreamHandlerBuilder::stderr())]
    fn reject_unknown_formatter(#[case] builder: StreamHandlerBuilder) {
        let builder = builder.with_formatter("does-not-exist");
        assert_build_err(&builder, "build_inner must fail for unknown formatter");
    }
}
