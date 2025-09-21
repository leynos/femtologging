//! Shared builder options.
//!
//! Stores fields common to multiple handler builders.

use std::num::NonZeroUsize;

#[cfg(feature = "python")]
use pyo3::{prelude::*, types::PyDict, Bound};

use super::{
    file::{HandlerConfig, OverflowPolicy},
    FormatterId, HandlerBuildError,
};

#[derive(Clone, Debug, Default)]
pub struct CommonBuilder {
    pub(crate) capacity: Option<NonZeroUsize>,
    pub(crate) capacity_set: bool,
    pub(crate) flush_timeout_ms: Option<u64>,
    pub(crate) formatter_id: Option<FormatterId>,
}

impl CommonBuilder {
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

    /// Extend a Python dictionary with common builder fields.
    #[cfg(feature = "python")]
    pub(crate) fn extend_py_dict(&self, d: &Bound<'_, PyDict>) -> PyResult<()> {
        if let Some(cap) = self.capacity {
            d.set_item("capacity", cap.get())?;
        }
        if let Some(ms) = self.flush_timeout_ms {
            d.set_item("flush_timeout_ms", ms)?;
        }
        if let Some(fid) = &self.formatter_id {
            d.set_item("formatter_id", fid.as_str())?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub(crate) struct FileLikeBuilderState {
    pub(crate) common: CommonBuilder,
    pub(crate) flush_record_interval: Option<usize>,
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
            flush_record_interval: None,
            overflow_policy: OverflowPolicy::Drop,
        }
    }

    /// Update the bounded channel capacity in place.
    pub(crate) fn set_capacity(&mut self, capacity: usize) {
        self.common.capacity = NonZeroUsize::new(capacity);
        self.common.capacity_set = true;
    }

    /// Update the flush interval in place.
    pub(crate) fn set_flush_record_interval(&mut self, interval: usize) {
        self.flush_record_interval = Some(interval);
    }

    /// Update the formatter identifier in place.
    pub(crate) fn set_formatter(&mut self, formatter_id: impl Into<FormatterId>) {
        self.common.formatter_id = Some(formatter_id.into());
    }

    /// Update the overflow policy in place.
    pub(crate) fn set_overflow_policy(&mut self, policy: OverflowPolicy) {
        self.overflow_policy = policy;
    }

    /// Validate queue-related settings shared between file-based builders.
    pub(crate) fn validate(&self) -> Result<(), HandlerBuildError> {
        self.common.is_capacity_valid()?;
        CommonBuilder::ensure_non_zero(
            "flush_record_interval",
            self.flush_record_interval.map(|value| value as u64),
        )?;
        if let OverflowPolicy::Timeout(duration) = self.overflow_policy {
            if duration.is_zero() {
                return Err(HandlerBuildError::InvalidConfig(
                    "timeout_ms must be greater than zero".into(),
                ));
            }
        }
        Ok(())
    }

    /// Produce a [`HandlerConfig`] populated with the configured values.
    pub(crate) fn handler_config(&self) -> HandlerConfig {
        let mut cfg = HandlerConfig::default();
        if let Some(capacity) = self.common.capacity {
            cfg.capacity = capacity.get();
        }
        if let Some(interval) = self.flush_record_interval {
            cfg.flush_interval = interval;
        }
        cfg.overflow_policy = self.overflow_policy;
        cfg
    }

    /// Expose the configured formatter identifier, if any.
    pub(crate) fn formatter_id(&self) -> Option<&FormatterId> {
        self.common.formatter_id.as_ref()
    }

    /// Extend a Python dictionary with shared file builder fields.
    #[cfg(feature = "python")]
    pub(crate) fn extend_py_dict(&self, d: &Bound<'_, PyDict>) -> PyResult<()> {
        self.common.extend_py_dict(d)?;
        if let Some(flush) = self.flush_record_interval {
            d.set_item("flush_record_interval", flush)?;
        }
        match self.overflow_policy {
            OverflowPolicy::Drop => d.set_item("overflow_policy", "drop")?,
            OverflowPolicy::Block => d.set_item("overflow_policy", "block")?,
            OverflowPolicy::Timeout(duration) => {
                d.set_item("timeout_ms", duration.as_millis() as u64)?;
                d.set_item("overflow_policy", "timeout")?;
            }
        }
        Ok(())
    }
}
