//! Builder for [`FemtoRotatingFileHandler`].
//!
//! Extends the file handler builder with rotation-specific parameters such as
//! ``max_bytes`` and ``backup_count``.

use std::num::{NonZeroU64, NonZeroUsize};

#[cfg(feature = "python")]
use pyo3::prelude::*;

use super::{
    common::FileLikeBuilderState,
    file::OverflowPolicy,
    rotating::{FemtoRotatingFileHandler, RotationConfig},
    FormatterId, HandlerBuildError, HandlerBuilderTrait,
};
use crate::formatter::DefaultFormatter;

use crate::handlers::builder_macros::{builder_methods, builder_methods_with_capacity};
#[cfg(feature = "python")]
use crate::macros::{dict_into_py, AsPyDict};

/// Builder for constructing [`FemtoRotatingFileHandler`] instances.
#[cfg_attr(feature = "python", pyclass)]
#[derive(Clone, Debug)]
pub struct RotatingFileHandlerBuilder {
    path: String,
    state: FileLikeBuilderState,
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
            state: FileLikeBuilderState::default(),
            max_bytes: None,
            max_bytes_set: false,
            backup_count: None,
            backup_count_set: false,
        }
    }

    /// Set the overflow policy for the handler.
    pub fn with_overflow_policy(mut self, policy: OverflowPolicy) -> Self {
        self.state.set_overflow_policy(policy);
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
        self.state.validate()?;
        self.ensure_rotation_limits_valid()
    }
}

#[cfg(feature = "python")]
impl RotatingFileHandlerBuilder {
    /// Populate a Python dictionary with the builder's fields.
    fn fill_pydict(&self, d: &pyo3::Bound<'_, pyo3::types::PyDict>) -> PyResult<()> {
        d.set_item("path", &self.path)?;
        self.state.extend_py_dict(d)?;
        d.set_item("max_bytes", self.max_bytes.map_or(0, NonZeroU64::get))?;
        d.set_item(
            "backup_count",
            self.backup_count.map_or(0, NonZeroUsize::get),
        )?;
        Ok(())
    }
}

builder_methods_with_capacity! {
    impl RotatingFileHandlerBuilder {
        capacity(
            self_ident = builder,
            setter = |builder, capacity| {
                builder.state.set_capacity(capacity);
            }
        );
        methods {
            method {
                doc: "Set the periodic flush interval measured in records.\n\n# Validation\n\nThe interval must be greater than zero; invalid values cause `build` to error.",
                rust_name: with_flush_record_interval,
                py_fn: py_with_flush_record_interval,
                py_name: "with_flush_record_interval",
                py_text_signature: "(self, interval)",
                rust_args: (interval: usize),
                self_ident: builder,
                body: {
                    builder.state.set_flush_record_interval(interval);
                }
            }
            method {
                doc: "Set the formatter identifier.",
                rust_name: with_formatter,
                py_fn: py_with_formatter,
                py_name: "with_formatter",
                py_text_signature: "(self, formatter_id)",
                rust_args: (formatter_id: impl Into<FormatterId>),
                py_args: (formatter_id: String),
                self_ident: builder,
                body: {
                    builder.state.set_formatter(formatter_id);
                }
            }
            method {
                doc: "Set the maximum number of bytes before rotation occurs.",
                rust_name: with_max_bytes,
                py_fn: py_with_max_bytes,
                py_name: "with_max_bytes",
                py_text_signature: "(self, max_bytes)",
                rust_args: (max_bytes: u64),
                py_args: (max_bytes: u64),
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
                py_args: (backup_count: usize),
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
                policy: &str,
                timeout_ms: Option<u64>,
            ) -> PyResult<PyRefMut<'py, Self>> {
                let policy_value = super::file::policy::parse_policy_with_timeout(policy, timeout_ms)?;
                slf.state.set_overflow_policy(policy_value);
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
        let cfg = self.state.handler_config();
        let rotation = match self.max_bytes {
            Some(max_bytes) => RotationConfig::new(
                max_bytes.get(),
                self.backup_count
                    .expect("validation ensures backup_count is set when max_bytes is set")
                    .get(),
            ),
            None => RotationConfig::disabled(),
        };
        match self.state.formatter_id() {
            Some(FormatterId::Default) | None => {
                let handler = FemtoRotatingFileHandler::with_capacity_flush_policy(
                    &self.path,
                    DefaultFormatter,
                    cfg,
                    rotation,
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
            Some(FormatterId::Custom(other)) => Err(HandlerBuildError::InvalidConfig(format!(
                "unknown formatter id: {other}",
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::assert_build_err;
    use super::*;
    use rstest::rstest;
    use tempfile::tempdir;

    #[rstest]
    fn build_rotating_file_handler_defaults() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.log");
        let builder = RotatingFileHandlerBuilder::new(path.to_string_lossy());
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
        let builder = RotatingFileHandlerBuilder::new(path.to_string_lossy())
            .with_capacity(32)
            .with_flush_record_interval(2)
            .with_max_bytes(1024)
            .with_backup_count(3);
        let mut handler = builder
            .build_inner()
            .expect("build_inner must succeed for valid rotation config");
        assert_eq!(handler.rotation_limits(), (1024, 3));
        handler.close();
    }

    #[rstest]
    fn reject_zero_capacity() {
        let builder = RotatingFileHandlerBuilder::new("log.txt").with_capacity(0);
        assert_build_err(&builder, "build_inner must fail for zero capacity");
    }

    #[rstest]
    fn reject_zero_flush_record_interval() {
        let builder = RotatingFileHandlerBuilder::new("log.txt").with_flush_record_interval(0);
        assert_build_err(
            &builder,
            "build_inner must fail for zero flush record interval",
        );
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
