//! Shared builder options.
//!
//! Stores fields common to multiple handler builders.

use std::num::NonZeroUsize;

#[derive(Clone, Debug, Default)]
pub struct CommonBuilder {
    pub(crate) capacity: Option<NonZeroUsize>,
    pub(crate) capacity_set: bool,
}

impl CommonBuilder {
    pub(crate) fn ensure_non_zero(
        field: &str,
        value: Option<u64>,
    ) -> Result<(), super::HandlerBuildError> {
        match value {
            Some(0) => Err(super::HandlerBuildError::InvalidConfig(format!(
                "{field} must be greater than zero",
            ))),
            _ => Ok(()),
        }
    }

    pub(crate) fn ensure_non_zero_u64(
        field: &str,
        value: Option<u64>,
    ) -> Result<(), super::HandlerBuildError> {
        Self::ensure_non_zero(field, value)
    }

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
