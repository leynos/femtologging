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
