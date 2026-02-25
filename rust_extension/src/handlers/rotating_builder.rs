//! Builder for [`FemtoRotatingFileHandler`].
//!
//! Extends the file handler builder with rotation-specific parameters such as
//! ``max_bytes`` and ``backup_count``.

use std::num::{NonZeroU64, NonZeroUsize};

use super::{
    FormatterId, HandlerBuildError, HandlerBuilderTrait,
    common::{FileLikeBuilderState, FormatterConfig, IntoFormatterConfig},
    file::{HandlerConfig, OverflowPolicy},
    rotating::{FemtoRotatingFileHandler, RotationConfig},
};
use crate::formatter::{DefaultFormatter, FemtoFormatter};

/// Builder for constructing [`FemtoRotatingFileHandler`] instances.
#[cfg_attr(feature = "python", pyo3::pyclass(from_py_object))]
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

    /// Set the bounded channel capacity.
    ///
    /// # Validation
    ///
    /// The capacity must be greater than zero; invalid values cause `build` to
    /// error.
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.common.set_capacity(capacity);
        self
    }

    /// Set the flush threshold measured in records.
    ///
    /// # Validation
    ///
    /// The threshold must be greater than zero (`NonZeroU64`).
    ///
    /// # Platform-specific behaviour
    ///
    /// On 32-bit platforms where `usize::MAX < u64::MAX`, values exceeding
    /// `usize::MAX` are clamped silently at build time. Python callers receive
    /// an `OverflowError` instead (validated at the API boundary).
    pub fn with_flush_after_records(mut self, interval: NonZeroU64) -> Self {
        self.common.set_flush_after_records(interval);
        self
    }

    /// Set the maximum number of bytes before rotation occurs.
    pub fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = NonZeroU64::new(max_bytes);
        self.max_bytes_set = true;
        self
    }

    /// Set how many backup files to retain during rotation.
    pub fn with_backup_count(mut self, backup_count: usize) -> Self {
        self.backup_count = NonZeroUsize::new(backup_count);
        self.backup_count_set = true;
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
mod python_bindings;

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
        let dir = tempdir().expect("tempdir must create a temporary directory");
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
        let dir = tempdir().expect("tempdir must create a temporary directory");
        let path = dir.path().join("test.log");
        let builder = RotatingFileHandlerBuilder::new(path.to_string_lossy().into_owned())
            .with_capacity(32)
            .with_flush_after_records(NonZeroU64::new(2).expect("2 is non-zero"))
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
        let dir = tempdir().expect("tempdir must create a temporary directory");
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
