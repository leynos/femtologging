//! Builder for [`FemtoFileHandler`].
//!
//! Provides a fluent API for configuring a file-based logging handler.
//! Only a subset of options are currently supported; additional
//! parameters such as encoding and mode will be added as the project
//! evolves. Flushing is driven by a `flush_after_records` threshold
//! measured in records.
//!
//! **Note:** Only the "default" `formatter_id` is currently supported.
//! Non-default identifiers will produce a build error. A formatter
//! registry will be wired in future to resolve custom identifiers at
//! build time.

#[cfg(feature = "python")]
use pyo3::prelude::*;

use std::num::NonZeroU64;
use std::path::PathBuf;

#[cfg(feature = "python")]
use super::common::{PyOverflowPolicy, py_flush_after_records_to_nonzero};
use super::{
    FormatterId, HandlerBuildError, HandlerBuilderTrait,
    common::{FileLikeBuilderState, FormatterConfig, IntoFormatterConfig},
    file::{FemtoFileHandler, OverflowPolicy},
};
use crate::formatter::DefaultFormatter;
#[cfg(test)]
use crate::level::FemtoLevel;

#[cfg(feature = "python")]
use crate::macros::{AsPyDict, dict_into_py};

/// Builder for constructing [`FemtoFileHandler`] instances.
#[cfg_attr(feature = "python", pyclass(from_py_object))]
#[derive(Clone, Debug)]
pub struct FileHandlerBuilder {
    path: PathBuf,
    common: FileLikeBuilderState,
}

impl FileHandlerBuilder {
    /// Create a builder targeting the specified file path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            common: FileLikeBuilderState::default(),
        }
    }

    /// Set the overflow policy for the handler.
    #[must_use]
    pub const fn with_overflow_policy(mut self, policy: OverflowPolicy) -> Self {
        self.common.set_overflow_policy(policy);
        self
    }

    /// Attach a formatter instance or identifier.
    #[must_use]
    pub fn with_formatter<F>(mut self, formatter: F) -> Self
    where
        F: IntoFormatterConfig,
    {
        self.common.set_formatter(formatter);
        self
    }
}

#[cfg(feature = "python")]
impl FileHandlerBuilder {
    /// Populate a Python dictionary with the builder's fields.
    fn fill_pydict(&self, d: &pyo3::Bound<'_, pyo3::types::PyDict>) -> PyResult<()> {
        let path = self.path.to_string_lossy();
        d.set_item("path", path.as_ref())?;
        self.common.extend_py_dict(d)?;
        Ok(())
    }
}

#[path = "file_builder_python_bindings.rs"]
mod python_bindings;

#[cfg(feature = "python")]
impl AsPyDict for FileHandlerBuilder {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
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
        self.common.validate()?;
        let cfg = self.common.handler_config();
        let handler = match self.common.formatter() {
            Some(FormatterConfig::Instance(fmt)) => {
                FemtoFileHandler::with_capacity_flush_policy(&self.path, fmt.clone_arc(), cfg)?
            }
            Some(FormatterConfig::Id(FormatterId::Default)) | None => {
                FemtoFileHandler::with_capacity_flush_policy(&self.path, DefaultFormatter, cfg)?
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
mod tests {
    //! Tests for the file handler builder.

    use super::super::test_helpers::assert_build_err;
    use super::*;
    use rstest::rstest;
    use tempfile::tempdir;

    use crate::{
        formatter::FemtoFormatter, handler::FemtoHandlerTrait, log_record::FemtoLogRecord,
    };

    #[derive(Clone, Copy, Debug)]
    struct PrefixFormatter;

    impl FemtoFormatter for PrefixFormatter {
        fn format(&self, record: &FemtoLogRecord) -> String {
            format!("prefix:{}", record.message())
        }
    }

    #[rstest]
    fn build_file_handler() {
        let dir = tempdir().expect("tempdir must create a temporary directory");
        let path = dir.path().join("test.log");
        let builder = FileHandlerBuilder::new(path.to_string_lossy().into_owned())
            .with_capacity(16)
            .with_flush_after_records(NonZeroU64::new(1).expect("1 is non-zero"));
        let handler = builder
            .build_inner()
            .expect("build_inner must succeed for a valid file builder");
        assert!(
            handler.flush(),
            "the built handler must acknowledge flushing"
        );
    }

    #[rstest]
    fn build_file_handler_with_custom_formatter() {
        let dir = tempdir().expect("tempdir must create a temporary directory");
        let path = dir.path().join("custom.log");
        let builder = FileHandlerBuilder::new(path.to_string_lossy().into_owned())
            .with_formatter(PrefixFormatter)
            .with_flush_after_records(NonZeroU64::new(1).expect("1 is non-zero"));
        let mut handler = builder
            .build_inner()
            .expect("build_inner must support custom formatter instances");
        handler
            .handle(FemtoLogRecord::new("logger", FemtoLevel::Info, "hello"))
            .expect("custom formatter write must succeed");
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
        let builder = FileHandlerBuilder::new("log.txt").with_capacity(0);
        assert_build_err(&builder, "build_inner must fail for zero capacity");
    }

    #[rstest]
    fn reject_zero_overflow_timeout() {
        let builder = FileHandlerBuilder::new("log.txt")
            .with_overflow_policy(OverflowPolicy::Timeout(std::time::Duration::from_millis(0)));
        assert_build_err(&builder, "build_inner must fail for zero timeout_ms");
    }
}
