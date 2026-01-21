//! Builder for [`FemtoRotatingFileHandler`].
//!
//! Extends the file handler builder with rotation-specific parameters such as
//! ``max_bytes`` and ``backup_count``.

use std::num::{NonZeroU64, NonZeroUsize};

#[cfg(feature = "python")]
use pyo3::prelude::*;

#[cfg(feature = "python")]
use super::common::{PyOverflowPolicy, py_flush_record_interval_to_nonzero};
use super::{
    FormatterId, HandlerBuildError, HandlerBuilderTrait,
    common::{FileLikeBuilderState, FormatterConfig, IntoFormatterConfig},
    file::{HandlerConfig, OverflowPolicy},
    rotating::{FemtoRotatingFileHandler, RotationConfig},
};
use crate::formatter::{DefaultFormatter, FemtoFormatter};

use crate::handlers::builder_macros::builder_methods;
#[cfg(feature = "python")]
use crate::macros::{AsPyDict, dict_into_py};

/// Builder for constructing [`FemtoRotatingFileHandler`] instances.
#[cfg_attr(feature = "python", pyclass)]
#[derive(Clone, Debug)]
pub struct RotatingFileHandlerBuilder {
    path: String,
    common: FileLikeBuilderState,
    max_bytes: Option<NonZeroU64>,
    max_bytes_set: bool,
    backup_count: Option<NonZeroUsize>,
    backup_count_set: bool,
}

impl RotatingFileHandlerBuilder {
    /// Create a builder targeting the specified file path.
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            common: FileLikeBuilderState::default(),
            max_bytes: None,
            max_bytes_set: false,
            backup_count: None,
            backup_count_set: false,
        }
    }

    /// Set the overflow policy for the handler.
    pub fn with_overflow_policy(mut self, policy: OverflowPolicy) -> Self {
        self.common.set_overflow_policy(policy);
        self
    }

    /// Attach a formatter instance or identifier.
    pub fn with_formatter<F>(mut self, formatter: F) -> Self
    where
        F: IntoFormatterConfig,
    {
        self.common.set_formatter(formatter);
        self
    }

    fn ensure_rotation_limits_valid(&self) -> Result<(), HandlerBuildError> {
        if self.max_bytes_set && self.max_bytes.is_none() {
            return Err(HandlerBuildError::InvalidConfig(
                "max_bytes must be greater than zero".into(),
            ));
        }
        if self.backup_count_set && self.backup_count.is_none() {
            return Err(HandlerBuildError::InvalidConfig(
                "backup_count must be greater than zero".into(),
            ));
        }
        match (self.max_bytes, self.backup_count) {
            (Some(_), Some(_)) | (None, None) => Ok(()),
            (Some(_), None) => Err(HandlerBuildError::InvalidConfig(
                "backup_count must be provided when max_bytes is set".into(),
            )),
            (None, Some(_)) => Err(HandlerBuildError::InvalidConfig(
                "max_bytes must be provided when backup_count is set".into(),
            )),
        }
    }

    fn validate(&self) -> Result<(), HandlerBuildError> {
        self.common.validate()?;
        self.ensure_rotation_limits_valid()
    }

    fn build_handler_with_formatter<F>(
        &self,
        formatter: F,
        cfg: HandlerConfig,
        rotation: RotationConfig,
    ) -> Result<FemtoRotatingFileHandler, HandlerBuildError>
    where
        F: FemtoFormatter + Send + 'static,
    {
        let handler = FemtoRotatingFileHandler::with_capacity_flush_policy(
            &self.path, formatter, cfg, rotation,
        )?;
        let limits = handler.rotation_limits();
        debug_assert_eq!(
            limits,
            (
                self.max_bytes.map_or(0, NonZeroU64::get),
                self.backup_count.map_or(0, NonZeroUsize::get),
            )
        );
        let _ = limits;
        Ok(handler)
    }
}

#[cfg(feature = "python")]
impl RotatingFileHandlerBuilder {
    /// Populate a Python dictionary with the builder's fields.
    fn fill_pydict(&self, d: &pyo3::Bound<'_, pyo3::types::PyDict>) -> PyResult<()> {
        d.set_item("path", &self.path)?;
        self.common.extend_py_dict(d)?;
        d.set_item("max_bytes", self.max_bytes.map_or(0, NonZeroU64::get))?;
        d.set_item(
            "backup_count",
            self.backup_count.map_or(0, NonZeroUsize::get),
        )?;
        Ok(())
    }
}

builder_methods! {
    impl RotatingFileHandlerBuilder {
        capacity {
            self_ident = builder,
            setter = |builder_ref, capacity| {
                builder_ref.common.set_capacity(capacity);
            }
        };
        methods {
            method {
                doc: "Set the periodic flush interval measured in records.\n\n# Validation\n\nAccepts a `NonZeroU64` so both Rust and Python callers must provide an interval greater than zero. Values exceeding `usize::MAX` on the current platform are rejected.",
                rust_name: with_flush_record_interval,
                py_fn: py_with_flush_record_interval,
                py_name: "with_flush_record_interval",
                py_text_signature: "(self, interval)",
                rust_args: (interval: NonZeroU64),
                py_args: (interval: u64),
                py_prelude: {
                    let interval = py_flush_record_interval_to_nonzero(
                        interval,
                        "flush_record_interval",
                    )?;
                },
                self_ident: builder,
                body: {
                    builder.common.set_flush_record_interval(interval);
                }
            }
            method {
                doc: "Set the maximum number of bytes before rotation occurs.",
                rust_name: with_max_bytes,
                py_fn: py_with_max_bytes,
                py_name: "with_max_bytes",
                py_text_signature: "(self, max_bytes)",
                rust_args: (max_bytes: u64),
                py_args: (max_bytes: pyo3::Bound<'py, pyo3::types::PyAny>),
                py_prelude: {
                    use pyo3::exceptions::{PyOverflowError, PyValueError};

                    let max_bytes = max_bytes.extract::<i128>()?;
                    if max_bytes < 0 {
                        return Err(PyValueError::new_err(
                            "max_bytes must be greater than zero",
                        ));
                    }
                    if max_bytes == 0 {
                        return Err(PyValueError::new_err(
                            "max_bytes must be greater than zero",
                        ));
                    }
                    let max_bytes = u64::try_from(max_bytes)
                        .map_err(|_| PyOverflowError::new_err(
                            "max_bytes exceeds the allowable range",
                        ))?;
                },
                self_ident: builder,
                body: {
                    builder.max_bytes = NonZeroU64::new(max_bytes);
                    builder.max_bytes_set = true;
                }
            }
            method {
                doc: "Set how many backup files to retain during rotation.",
                rust_name: with_backup_count,
                py_fn: py_with_backup_count,
                py_name: "with_backup_count",
                py_text_signature: "(self, backup_count)",
                rust_args: (backup_count: usize),
                py_args: (backup_count: pyo3::Bound<'py, pyo3::types::PyAny>),
                py_prelude: {
                    use pyo3::exceptions::{PyOverflowError, PyValueError};

                    let backup_count = backup_count.extract::<i128>()?;
                    if backup_count <= 0 {
                        return Err(PyValueError::new_err(
                            "backup_count must be greater than zero",
                        ));
                    }
                    let backup_count = usize::try_from(backup_count)
                        .map_err(|_| PyOverflowError::new_err(
                            "backup_count exceeds the allowable range",
                        ))?;
                },
                self_ident: builder,
                body: {
                    builder.backup_count = NonZeroUsize::new(backup_count);
                    builder.backup_count_set = true;
                }
            }
        }
        extra_py_methods {
            /// Create a new `RotatingFileHandlerBuilder`.
            #[new]
            fn py_new(path: String) -> Self {
                Self::new(path)
            }

            #[pyo3(name = "with_overflow_policy")]
            fn py_with_overflow_policy<'py>(
                mut slf: PyRefMut<'py, Self>,
                policy: PyOverflowPolicy,
            ) -> PyResult<PyRefMut<'py, Self>> {
                slf.common.set_overflow_policy(policy.inner);
                Ok(slf)
            }

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

            /// Return a dictionary describing the builder configuration.
            fn as_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
                self.as_pydict(py)
            }

            /// Build the handler, raising ``HandlerConfigError`` or ``HandlerIOError`` on
            /// failure.
            fn build(&self) -> PyResult<FemtoRotatingFileHandler> {
                <Self as HandlerBuilderTrait>::build_inner(self).map_err(PyErr::from)
            }
        }
    }
}

#[cfg(feature = "python")]
impl AsPyDict for RotatingFileHandlerBuilder {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let d = pyo3::types::PyDict::new(py);
        self.fill_pydict(&d)?;
        dict_into_py(d, py)
    }
}

impl HandlerBuilderTrait for RotatingFileHandlerBuilder {
    type Handler = FemtoRotatingFileHandler;

    fn build_inner(&self) -> Result<Self::Handler, HandlerBuildError> {
        self.validate()?;
        let cfg = self.common.handler_config();
        let rotation = match self.max_bytes {
            Some(max_bytes) => RotationConfig::new(
                max_bytes.get(),
                self.backup_count
                    .expect("validation ensures backup_count is set when max_bytes is set")
                    .get(),
            ),
            None => RotationConfig::disabled(),
        };
        match self.common.formatter() {
            Some(FormatterConfig::Instance(fmt)) => {
                self.build_handler_with_formatter(fmt.clone_arc(), cfg, rotation)
            }
            Some(FormatterConfig::Id(FormatterId::Default)) | None => {
                self.build_handler_with_formatter(DefaultFormatter, cfg, rotation)
            }
            Some(FormatterConfig::Id(FormatterId::Custom(other))) => Err(
                HandlerBuildError::InvalidConfig(format!("unknown formatter id: {other}",)),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::assert_build_err;
    use super::*;
    use rstest::rstest;
    use tempfile::tempdir;

    use crate::{formatter::FemtoFormatter, log_record::FemtoLogRecord};

    #[derive(Clone, Copy, Debug)]
    struct SuffixFormatter;

    impl FemtoFormatter for SuffixFormatter {
        fn format(&self, record: &FemtoLogRecord) -> String {
            format!("suffix:{}", record.message())
        }
    }

    #[rstest]
    fn build_rotating_file_handler_defaults() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.log");
        let builder = RotatingFileHandlerBuilder::new(path.to_string_lossy().into_owned());
        let mut handler = builder
            .build_inner()
            .expect("build_inner must succeed for defaults");
        assert_eq!(handler.rotation_limits(), (0, 0));
        handler.close();
    }

    #[rstest]
    fn build_rotating_file_handler_with_limits() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.log");
        let builder = RotatingFileHandlerBuilder::new(path.to_string_lossy().into_owned())
            .with_capacity(32)
            .with_flush_record_interval(NonZeroU64::new(2).expect("2 is non-zero"))
            .with_max_bytes(1024)
            .with_backup_count(3);
        let mut handler = builder
            .build_inner()
            .expect("build_inner must succeed for valid rotation config");
        assert_eq!(handler.rotation_limits(), (1024, 3));
        handler.close();
    }

    #[rstest]
    fn build_rotating_file_handler_with_custom_formatter() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.log");
        let builder = RotatingFileHandlerBuilder::new(path.to_string_lossy().into_owned())
            .with_formatter(SuffixFormatter)
            .with_capacity(8);
        let mut handler = builder
            .build_inner()
            .expect("build_inner must accept formatter instances");
        handler.flush();
        handler.close();
    }

    #[rstest]
    fn reject_zero_capacity() {
        let builder = RotatingFileHandlerBuilder::new("log.txt").with_capacity(0);
        assert_build_err(&builder, "build_inner must fail for zero capacity");
    }

    #[rstest]
    fn reject_zero_overflow_timeout() {
        let builder = RotatingFileHandlerBuilder::new("log.txt")
            .with_overflow_policy(OverflowPolicy::Timeout(std::time::Duration::from_millis(0)));
        assert_build_err(&builder, "build_inner must fail for zero timeout_ms");
    }
    #[rstest]
    fn reject_zero_max_bytes() {
        let builder = RotatingFileHandlerBuilder::new("log.txt")
            .with_max_bytes(0)
            .with_backup_count(1);
        assert_build_err(&builder, "build_inner must fail for zero max_bytes");
    }

    #[rstest]
    fn reject_zero_backup_count() {
        let builder = RotatingFileHandlerBuilder::new("log.txt")
            .with_max_bytes(1024)
            .with_backup_count(0);
        assert_build_err(&builder, "build_inner must fail for zero backup_count");
    }

    #[rstest]
    fn reject_missing_backup_count() {
        let builder = RotatingFileHandlerBuilder::new("log.txt").with_max_bytes(1024);
        assert_build_err(
            &builder,
            "build_inner must fail when backup_count is missing",
        );
    }

    #[rstest]
    fn reject_missing_max_bytes() {
        let builder = RotatingFileHandlerBuilder::new("log.txt").with_backup_count(2);
        assert_build_err(&builder, "build_inner must fail when max_bytes is missing");
    }
}
