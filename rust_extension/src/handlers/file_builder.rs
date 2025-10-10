//! Builder for [`FemtoFileHandler`].
//!
//! Provides a fluent API for configuring a file-based logging handler.
//! Only a subset of options are currently supported; additional
//! parameters such as encoding and mode will be added as the project
//! evolves. Flushing is driven by a `flush_record_interval`
//! measured in records.

#[cfg(feature = "python")]
use pyo3::{exceptions::PyValueError, prelude::*};

use std::path::PathBuf;

#[cfg(feature = "python")]
use super::file::policy::parse_policy_with_timeout;
use super::{
    common::{FileLikeBuilderState, FormatterConfig, IntoFormatterConfig},
    file::{FemtoFileHandler, OverflowPolicy},
    FormatterId, HandlerBuildError, HandlerBuilderTrait,
};
use crate::formatter::DefaultFormatter;

use crate::handlers::builder_macros::builder_methods;
#[cfg(feature = "python")]
use crate::macros::{dict_into_py, AsPyDict};

/// Builder for constructing [`FemtoFileHandler`] instances.
#[cfg_attr(feature = "python", pyclass)]
#[derive(Clone, Debug)]
pub struct FileHandlerBuilder {
    path: PathBuf,
    state: FileLikeBuilderState,
}

impl FileHandlerBuilder {
    /// Create a builder targeting the specified file path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            state: FileLikeBuilderState::default(),
        }
    }

    /// Set the overflow policy for the handler.
    pub fn with_overflow_policy(mut self, policy: OverflowPolicy) -> Self {
        self.state.set_overflow_policy(policy);
        self
    }

    /// Attach a formatter instance or identifier.
    pub fn with_formatter<F>(mut self, formatter: F) -> Self
    where
        F: IntoFormatterConfig,
    {
        self.state.set_formatter(formatter);
        self
    }
}

#[cfg(feature = "python")]
impl FileHandlerBuilder {
    /// Populate a Python dictionary with the builder's fields.
    fn fill_pydict(&self, d: &pyo3::Bound<'_, pyo3::types::PyDict>) -> PyResult<()> {
        let path = self.path.to_string_lossy();
        d.set_item("path", path.as_ref())?;
        self.state.extend_py_dict(d)?;
        Ok(())
    }

    fn set_formatter_from_py(&mut self, formatter: &Bound<'_, PyAny>) -> PyResult<()> {
        match formatter.extract::<String>() {
            Ok(fid) => {
                self.state.set_formatter(fid);
                Ok(())
            }
            Err(string_err) => match crate::formatter::python::formatter_from_py(formatter) {
                Ok(instance) => {
                    self.state.set_formatter(instance);
                    Ok(())
                }
                Err(instance_err) => {
                    instance_err.set_cause(formatter.py(), Some(string_err));
                    Err(instance_err)
                }
            },
        }
    }
}

builder_methods! {
    impl FileHandlerBuilder {
        capacity {
            self_ident = builder,
            setter = |builder, capacity| {
                builder.state.set_capacity(capacity);
            }
        };
        methods {
            method {
                doc: "Set the periodic flush interval measured in records.\n\n# Validation\n\nThe interval must be greater than zero. Python callers receive ``ValueError``\nwhen the interval is zero; Rust callers observe a ``HandlerBuildError`` during\n``build``.",
                rust_name: with_flush_record_interval,
                py_fn: py_with_flush_record_interval,
                py_name: "with_flush_record_interval",
                py_text_signature: "(self, interval)",
                rust_args: (interval: usize),
                py_args: (interval: usize),
                py_prelude: {
                    if interval == 0 {
                        return Err(PyValueError::new_err(
                            "flush_record_interval must be greater than zero",
                        ));
                    }
                },
                self_ident: builder,
                body: {
                    builder.state.set_flush_record_interval(interval);
                }
            }
        }
        extra_py_methods {
            /// Create a new `FileHandlerBuilder`.
            ///
            /// Mirrors Python's `logging.FileHandler` constructor by accepting the
            /// filesystem path directly so Python callers can pass the same
            /// `filename` argument.
            #[new]
            fn py_new(path: String) -> Self {
                Self::new(path)
            }

            #[pyo3(name = "with_overflow_policy")]
            fn py_with_overflow_policy<'py>(
                mut slf: PyRefMut<'py, Self>,
                policy: &str,
                timeout_ms: Option<u64>,
            ) -> PyResult<PyRefMut<'py, Self>> {
                let policy_value = parse_policy_with_timeout(policy, timeout_ms)
                    .map_err(|err| PyValueError::new_err(err.to_string()))?;
                slf.state.set_overflow_policy(policy_value);
                Ok(slf)
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
            fn build(&self) -> PyResult<FemtoFileHandler> {
                <Self as HandlerBuilderTrait>::build_inner(self).map_err(PyErr::from)
            }
        }
    }
}

#[cfg(feature = "python")]
impl AsPyDict for FileHandlerBuilder {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let d = pyo3::types::PyDict::new(py);
        self.fill_pydict(&d)?;
        dict_into_py(d, py)
    }
}

impl HandlerBuilderTrait for FileHandlerBuilder {
    type Handler = FemtoFileHandler;

    /// Build a [`FemtoFileHandler`].
    ///
    /// `DEFAULT_CHANNEL_CAPACITY` (1024) when `with_capacity` is not called.
    fn build_inner(&self) -> Result<Self::Handler, HandlerBuildError> {
        self.state.validate()?;
        let cfg = self.state.handler_config();
        let handler = match self.state.formatter() {
            Some(FormatterConfig::Instance(fmt)) => {
                FemtoFileHandler::with_capacity_flush_policy(&self.path, fmt.clone(), cfg)?
            }
            Some(FormatterConfig::Id(FormatterId::Default)) | None => {
                FemtoFileHandler::with_capacity_flush_policy(&self.path, DefaultFormatter, cfg)?
            }
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
    use rstest::rstest;
    use std::path::PathBuf;
    use tempfile::tempdir;

    use crate::{
        formatter::FemtoFormatter, handler::FemtoHandlerTrait, log_record::FemtoLogRecord,
    };

    #[derive(Clone, Copy, Debug)]
    struct PrefixFormatter;

    impl FemtoFormatter for PrefixFormatter {
        fn format(&self, record: &FemtoLogRecord) -> String {
            format!("prefix:{}", record.message)
        }
    }

    #[rstest]
    fn build_file_handler() {
        let dir = tempdir().expect("tempdir must create a temporary directory");
        let path = dir.path().join("test.log");
        let builder = FileHandlerBuilder::new(path.clone())
            .with_capacity(16)
            .with_flush_record_interval(1);
        let handler = builder
            .build_inner()
            .expect("build_inner must succeed for a valid file builder");
        handler.flush();
    }

    #[rstest]
    fn build_file_handler_with_custom_formatter() {
        let dir = tempdir().expect("tempdir must create a temporary directory");
        let path = dir.path().join("custom.log");
        let builder = FileHandlerBuilder::new(path.to_string_lossy())
            .with_formatter(PrefixFormatter)
            .with_flush_record_interval(1);
        let mut handler = builder
            .build_inner()
            .expect("build_inner must support custom formatter instances");
        handler.handle(FemtoLogRecord::new("logger", "INFO", "hello"));
        assert!(handler.flush(), "flush must succeed for custom formatter");
        handler.close();

        let contents =
            std::fs::read_to_string(&path).expect("custom formatter must write formatted output");
        assert!(
            contents.contains("prefix:hello"),
            "custom formatter output must include prefix"
        );
    }

    #[rstest]
    fn reject_zero_capacity() {
        let builder = FileHandlerBuilder::new(PathBuf::from("log.txt")).with_capacity(0);
        assert_build_err(&builder, "build_inner must fail for zero capacity");
    }

    #[rstest]
    fn reject_zero_flush_record_interval() {
        let builder =
            FileHandlerBuilder::new(PathBuf::from("log.txt")).with_flush_record_interval(0);
        assert_build_err(
            &builder,
            "build_inner must fail for zero flush record interval",
        );
    }

    #[rstest]
    fn reject_zero_overflow_timeout() {
        let builder = FileHandlerBuilder::new(PathBuf::from("log.txt"))
            .with_overflow_policy(OverflowPolicy::Timeout(std::time::Duration::from_millis(0)));
        assert_build_err(&builder, "build_inner must fail for zero timeout_ms");
    }
}
