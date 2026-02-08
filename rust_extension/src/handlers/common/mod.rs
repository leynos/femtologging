//! Shared builder options.
//!
//! Stores fields common to multiple handler builders.

use std::{
    fmt,
    num::{NonZeroU64, NonZeroUsize},
};

use super::{
    FormatterId, HandlerBuildError,
    file::{HandlerConfig, OverflowPolicy},
};
use crate::formatter::{FemtoFormatter, SharedFormatter};

#[cfg(feature = "python")]
mod python;
#[cfg(feature = "python")]
pub use python::PyOverflowPolicy;
#[cfg(feature = "python")]
pub(crate) use python::py_flush_after_records_to_nonzero;

/// Formatter configuration stored by handler builders.
#[derive(Clone)]
pub enum FormatterConfig {
    /// Formatter referenced by identifier.
    Id(FormatterId),
    /// Formatter provided as an instance.
    Instance(SharedFormatter),
}

impl fmt::Debug for FormatterConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Id(id) => f.debug_tuple("Id").field(id).finish(),
            Self::Instance(_) => f.write_str("Instance(<formatter>)"),
        }
    }
}

/// Convert inputs into [`FormatterConfig`] values for builder consumption.
pub trait IntoFormatterConfig {
    /// Convert `self` into a [`FormatterConfig`].
    fn into_formatter_config(self) -> FormatterConfig;
}

impl<F> IntoFormatterConfig for F
where
    F: FemtoFormatter + 'static,
{
    fn into_formatter_config(self) -> FormatterConfig {
        FormatterConfig::Instance(SharedFormatter::new(self))
    }
}

impl IntoFormatterConfig for SharedFormatter {
    fn into_formatter_config(self) -> FormatterConfig {
        FormatterConfig::Instance(self)
    }
}

impl IntoFormatterConfig for FormatterId {
    fn into_formatter_config(self) -> FormatterConfig {
        FormatterConfig::Id(self)
    }
}

impl IntoFormatterConfig for String {
    fn into_formatter_config(self) -> FormatterConfig {
        FormatterId::from(self).into_formatter_config()
    }
}

impl IntoFormatterConfig for &str {
    fn into_formatter_config(self) -> FormatterConfig {
        FormatterId::from(self).into_formatter_config()
    }
}

#[derive(Clone, Debug, Default)]
pub struct CommonBuilder {
    pub(crate) capacity: Option<NonZeroUsize>,
    pub(crate) capacity_set: bool,
    pub(crate) flush_after_ms: Option<NonZeroU64>,
    pub(crate) formatter: Option<FormatterConfig>,
}

impl CommonBuilder {
    pub(crate) const DEFAULT_FLUSH_AFTER_MS: u64 = 1_000;

    /// Update the bounded channel capacity in place.
    ///
    /// A zero capacity is recorded for validation but does not update the
    /// stored [`NonZeroUsize`]. Callers rely on [`is_capacity_valid`] to surface
    /// the configuration error when `build` is invoked.
    pub(crate) fn set_capacity(&mut self, capacity: usize) {
        if capacity == 0 {
            self.capacity = None;
            self.capacity_set = true;
            return;
        }

        self.capacity = Some(
            NonZeroUsize::new(capacity)
                .expect("NonZeroUsize::new must succeed for non-zero capacity"),
        );
        self.capacity_set = true;
    }

    pub(crate) fn set_formatter<F>(&mut self, formatter: F)
    where
        F: IntoFormatterConfig,
    {
        self.formatter = Some(formatter.into_formatter_config());
    }

    /// Validate that an optional numeric field (if provided) is greater than zero.
    ///
    /// Returns `InvalidConfig("{field} must be greater than zero")` when `value`
    /// is `Some(0)`.
    pub(crate) fn ensure_non_zero(
        field: &str,
        value: Option<u64>,
    ) -> Result<(), super::HandlerBuildError> {
        match value {
            Some(0) => Err(super::HandlerBuildError::InvalidConfig(format!(
                "{field} must be greater than zero"
            ))),
            _ => Ok(()),
        }
    }

    /// Validate capacity semantics.
    ///
    /// When `capacity_set` is true and `capacity` is `None`, the caller
    /// attempted to set zero; return
    /// `InvalidConfig("capacity must be greater than zero")` in this case.
    pub(crate) fn is_capacity_valid(&self) -> Result<(), super::HandlerBuildError> {
        if self.capacity.is_none() && self.capacity_set {
            Err(super::HandlerBuildError::InvalidConfig(
                "capacity must be greater than zero".into(),
            ))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_capacity_stores_non_zero_value() {
        let mut builder = CommonBuilder::default();
        builder.set_capacity(32);

        let stored = builder
            .capacity
            .expect("set_capacity must store a NonZeroUsize for non-zero input");
        assert_eq!(stored.get(), 32);
        assert!(
            builder.capacity_set,
            "set_capacity must mark capacity as configured"
        );
    }

    #[test]
    fn set_capacity_zero_is_reported_invalid() {
        let mut builder = CommonBuilder::default();
        builder.set_capacity(0);

        assert!(
            builder.capacity.is_none(),
            "zero capacity must not store a value"
        );
        assert!(
            builder.capacity_set,
            "zero capacity must record that configuration was attempted"
        );
        let err = builder
            .is_capacity_valid()
            .expect_err("zero capacity must be rejected during validation");
        assert!(matches!(
            err,
            HandlerBuildError::InvalidConfig(message) if message == "capacity must be greater than zero"
        ));
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FileLikeBuilderState {
    pub(crate) common: CommonBuilder,
    pub(crate) flush_after_records: Option<NonZeroU64>,
    pub(crate) overflow_policy: OverflowPolicy,
}

impl Default for FileLikeBuilderState {
    fn default() -> Self {
        Self::new()
    }
}

impl FileLikeBuilderState {
    /// Create a new builder state with default queue settings.
    pub(crate) fn new() -> Self {
        Self {
            common: CommonBuilder::default(),
            flush_after_records: None,
            overflow_policy: OverflowPolicy::Drop,
        }
    }

    /// Update the bounded channel capacity in place.
    pub(crate) fn set_capacity(&mut self, capacity: usize) {
        self.common.set_capacity(capacity);
    }

    /// Update the flush threshold in place.
    ///
    /// Accepts a `NonZeroU64` to enforce the non-zero constraint at the type
    /// level, matching the `StreamHandlerBuilder` pattern for `flush_after_ms`.
    pub(crate) fn set_flush_after_records(&mut self, interval: NonZeroU64) {
        self.flush_after_records = Some(interval);
    }

    /// Update the formatter identifier in place.
    pub(crate) fn set_formatter<F>(&mut self, formatter: F)
    where
        F: IntoFormatterConfig,
    {
        self.common.set_formatter(formatter);
    }

    /// Update the overflow policy in place.
    pub(crate) fn set_overflow_policy(&mut self, policy: OverflowPolicy) {
        self.overflow_policy = policy;
    }

    /// Validate queue-related settings shared between file-based builders.
    ///
    /// The `flush_after_records` field uses `NonZeroU64`, so zero values are
    /// rejected at the type level and no explicit check is needed here.
    pub(crate) fn validate(&self) -> Result<(), HandlerBuildError> {
        self.common.is_capacity_valid()?;
        if let OverflowPolicy::Timeout(duration) = self.overflow_policy
            && duration.is_zero()
        {
            return Err(HandlerBuildError::InvalidConfig(
                "timeout_ms must be greater than zero".into(),
            ));
        }
        Ok(())
    }

    /// Produce a [`HandlerConfig`] populated with the configured values.
    ///
    /// The `flush_after_records` is stored as `NonZeroU64` but `HandlerConfig`
    /// uses `usize` internally. Python bindings reject values exceeding
    /// `usize::MAX` at the boundary via [`py_flush_after_records_to_nonzero`],
    /// so the conversion here is infallible for Python callers. Rust callers
    /// constructing oversized intervals directly will see clamping.
    pub(crate) fn handler_config(&self) -> HandlerConfig {
        let mut cfg = HandlerConfig::default();
        if let Some(capacity) = self.common.capacity {
            cfg.capacity = capacity.get();
        }
        if let Some(interval) = self.flush_after_records {
            // Python callers are validated at the boundary; Rust callers may
            // still pass large values directly, so clamp as a fallback.
            cfg.flush_interval = usize::try_from(interval.get()).unwrap_or(usize::MAX);
        }
        cfg.overflow_policy = self.overflow_policy;
        cfg
    }

    /// Expose the configured formatter identifier, if any.
    pub(crate) fn formatter(&self) -> Option<&FormatterConfig> {
        self.common.formatter.as_ref()
    }
}
