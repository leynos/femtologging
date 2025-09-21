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
#[cfg(feature = "python")]
use crate::macros::{dict_into_py, AsPyDict};

#[cfg(feature = "python")]
use super::common::CommonBuilder;

macro_rules! builder_state_method {
    ($doc:literal, $fn_name:ident, $arg:ident : $ty:ty, $with_method:ident) => {
        #[doc = $doc]
        pub fn $fn_name(mut self, $arg: $ty) -> Self {
            self.state = self.state.$with_method($arg);
            self
        }
    };
}

macro_rules! rotation_limit_method {
    ($doc:literal, $fn_name:ident, $value_field:ident, $flag_field:ident, $non_zero:ty, $arg:ident : $ty:ty) => {
        #[doc = $doc]
        pub fn $fn_name(mut self, $arg: $ty) -> Self {
            self.$value_field = <$non_zero>::new($arg);
            self.$flag_field = true;
            self
        }
    };
}

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

    builder_state_method!(
        "Set the bounded channel capacity.",
        with_capacity,
        capacity: usize,
        with_capacity
    );

    builder_state_method!(
        "Set the periodic flush interval measured in records. Must be greater than zero.",
        with_flush_record_interval,
        interval: usize,
        with_flush_record_interval
    );

    builder_state_method!(
        "Set the formatter identifier.",
        with_formatter,
        formatter_id: impl Into<FormatterId>,
        with_formatter
    );

    /// Set the overflow policy for the handler.
    pub fn with_overflow_policy(mut self, policy: OverflowPolicy) -> Self {
        self.state = self.state.with_overflow_policy(policy);
        self
    }

    rotation_limit_method!(
        "Set the maximum number of bytes before rotation occurs.",
        with_max_bytes,
        max_bytes,
        max_bytes_set,
        NonZeroU64,
        max_bytes: u64
    );

    rotation_limit_method!(
        "Set how many backup files to retain during rotation.",
        with_backup_count,
        backup_count,
        backup_count_set,
        NonZeroUsize,
        backup_count: usize
    );

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

#[cfg(feature = "python")]
impl AsPyDict for RotatingFileHandlerBuilder {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let d = pyo3::types::PyDict::new(py);
        self.fill_pydict(&d)?;
        dict_into_py(d, py)
    }
}

#[cfg(feature = "python")]
#[pymethods]
impl RotatingFileHandlerBuilder {
    #[new]
    fn py_new(path: String) -> Self {
        Self::new(path)
    }

    #[pyo3(name = "with_capacity")]
    fn py_with_capacity<'py>(mut slf: PyRefMut<'py, Self>, capacity: usize) -> PyRefMut<'py, Self> {
        slf.state.set_capacity(capacity);
        slf
    }

    #[pyo3(name = "with_flush_record_interval")]
    fn py_with_flush_record_interval<'py>(
        mut slf: PyRefMut<'py, Self>,
        interval: usize,
    ) -> PyRefMut<'py, Self> {
        slf.state.set_flush_record_interval(interval);
        slf
    }

    #[pyo3(name = "with_formatter")]
    fn py_with_formatter<'py>(
        mut slf: PyRefMut<'py, Self>,
        formatter_id: String,
    ) -> PyRefMut<'py, Self> {
        slf.state.set_formatter(formatter_id);
        slf
    }

    #[pyo3(name = "with_max_bytes")]
    fn py_with_max_bytes<'py>(mut slf: PyRefMut<'py, Self>, max_bytes: u64) -> PyRefMut<'py, Self> {
        slf.max_bytes = NonZeroU64::new(max_bytes);
        slf.max_bytes_set = true;
        slf
    }

    #[pyo3(name = "with_backup_count")]
    fn py_with_backup_count<'py>(
        mut slf: PyRefMut<'py, Self>,
        backup_count: usize,
    ) -> PyRefMut<'py, Self> {
        slf.backup_count = NonZeroUsize::new(backup_count);
        slf.backup_count_set = true;
        slf
    }

    #[pyo3(name = "with_overflow_policy")]
    fn py_with_overflow_policy<'py>(
        mut slf: PyRefMut<'py, Self>,
        policy: &str,
        timeout_ms: Option<u64>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let policy_value = if policy.eq_ignore_ascii_case("drop") {
            OverflowPolicy::Drop
        } else if policy.eq_ignore_ascii_case("block") {
            OverflowPolicy::Block
        } else if policy.eq_ignore_ascii_case("timeout") {
            let ms = timeout_ms.ok_or_else(|| {
                PyErr::new::<pyo3::exceptions::PyValueError, _>(
                    "timeout_ms required for timeout policy",
                )
            })?;
            CommonBuilder::ensure_non_zero("timeout_ms", Some(ms)).map_err(PyErr::from)?;
            OverflowPolicy::Timeout(std::time::Duration::from_millis(ms))
        } else {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "invalid overflow policy: {policy}",
            )));
        };
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
