//! Builder for [`FemtoTimedRotatingFileHandler`].
//!
//! Extends file-handler builder state with timed-rotation-specific settings.

use std::{num::NonZeroU64, path::PathBuf};

use chrono::NaiveTime;

use super::{
    FormatterId, HandlerBuildError, HandlerBuilderTrait,
    common::{FileLikeBuilderState, FormatterConfig, IntoFormatterConfig},
    file::{HandlerConfig, OverflowPolicy},
    timed_rotating::{
        FemtoTimedRotatingFileHandler, TimedRotationConfig, TimedRotationSchedule,
        TimedRotationWhen,
    },
};
use crate::formatter::{DefaultFormatter, FemtoFormatter};

/// Builder for constructing [`FemtoTimedRotatingFileHandler`] instances.
#[cfg_attr(feature = "python", pyo3::pyclass(from_py_object))]
#[derive(Clone, Debug)]
pub struct TimedRotatingFileHandlerBuilder {
    path: PathBuf,
    common: FileLikeBuilderState,
    when: TimedRotationWhen,
    interval: NonZeroU64,
    backup_count: usize,
    use_utc: bool,
    at_time: Option<NaiveTime>,
}

impl TimedRotatingFileHandlerBuilder {
    /// Create a builder targeting the specified file path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            common: FileLikeBuilderState::default(),
            when: TimedRotationWhen::Hours,
            interval: NonZeroU64::new(1).expect("1 is non-zero"),
            backup_count: 0,
            use_utc: false,
            at_time: None,
        }
    }

    /// Set the overflow policy for queue saturation.
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
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.common.set_capacity(capacity);
        self
    }

    /// Set the flush threshold measured in records.
    pub fn with_flush_after_records(mut self, interval: NonZeroU64) -> Self {
        self.common.set_flush_after_records(interval);
        self
    }

    fn validate_at_time_supported(&self) -> Result<(), HandlerBuildError> {
        if self.at_time.is_some() && !self.when.supports_at_time() {
            return Err(HandlerBuildError::InvalidConfig(format!(
                "at_time is only supported for daily, midnight, and weekday rotation (got {})",
                self.when.as_str(),
            )));
        }
        Ok(())
    }

    /// Set the timed rotation cadence.
    pub fn with_when(mut self, when: impl AsRef<str>) -> Result<Self, HandlerBuildError> {
        self.when =
            TimedRotationWhen::parse(when.as_ref()).map_err(HandlerBuildError::InvalidConfig)?;
        self.validate_at_time_supported()?;
        Ok(self)
    }

    /// Set the timed rotation interval.
    pub fn with_interval(mut self, interval: NonZeroU64) -> Self {
        self.interval = interval;
        self
    }

    /// Set how many timestamped backup files to retain.
    pub fn with_backup_count(mut self, backup_count: usize) -> Self {
        self.backup_count = backup_count;
        self
    }

    /// Select UTC instead of local time for schedule calculations.
    pub fn with_utc(mut self, use_utc: bool) -> Self {
        self.use_utc = use_utc;
        self
    }

    /// Set an explicit time-of-day trigger for eligible schedules.
    ///
    /// When called from Python, `datetime.time` microsecond precision is
    /// preserved internally. The getter formats the stored value with
    /// `NaiveTime::to_string`, which includes subsecond digits only when
    /// they are non-zero.
    pub fn with_at_time(mut self, at_time: Option<NaiveTime>) -> Result<Self, HandlerBuildError> {
        self.at_time = at_time;
        self.validate_at_time_supported()?;
        Ok(self)
    }

    fn schedule(&self) -> Result<TimedRotationSchedule, HandlerBuildError> {
        let interval = u32::try_from(self.interval.get()).map_err(|_| {
            HandlerBuildError::InvalidConfig("interval exceeds supported range".to_string())
        })?;
        TimedRotationSchedule::new(self.when, interval, self.use_utc, self.at_time)
            .map_err(HandlerBuildError::InvalidConfig)
    }

    fn validate(&self) -> Result<(), HandlerBuildError> {
        self.common.validate()?;
        let _ = self.schedule()?;
        Ok(())
    }

    fn build_handler_with_formatter<F>(
        &self,
        formatter: F,
        cfg: HandlerConfig,
        schedule: TimedRotationSchedule,
    ) -> Result<FemtoTimedRotatingFileHandler, HandlerBuildError>
    where
        F: FemtoFormatter + Send + 'static,
    {
        FemtoTimedRotatingFileHandler::with_capacity_flush_policy(
            &self.path,
            formatter,
            cfg,
            TimedRotationConfig {
                schedule,
                backup_count: self.backup_count,
            },
        )
        .map_err(Into::into)
    }
}

#[cfg(feature = "python")]
mod python_bindings;

impl HandlerBuilderTrait for TimedRotatingFileHandlerBuilder {
    type Handler = FemtoTimedRotatingFileHandler;

    fn build_inner(&self) -> Result<Self::Handler, HandlerBuildError> {
        self.validate()?;
        let cfg = self.common.handler_config();
        let schedule = self.schedule()?;
        match self.common.formatter() {
            Some(FormatterConfig::Instance(fmt)) => {
                self.build_handler_with_formatter(fmt.clone_arc(), cfg, schedule)
            }
            Some(FormatterConfig::Id(FormatterId::Default)) | None => {
                self.build_handler_with_formatter(DefaultFormatter, cfg, schedule)
            }
            Some(FormatterConfig::Id(FormatterId::Custom(other))) => Err(
                HandlerBuildError::InvalidConfig(format!("unknown formatter id: {other}",)),
            ),
        }
    }
}
