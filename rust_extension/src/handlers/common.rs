//! Shared builder options.
//!
//! Stores fields common to multiple handler builders.

#[derive(Clone, Debug, Default)]
pub struct CommonBuilder {
    pub(crate) capacity: Option<usize>,
}

impl CommonBuilder {
    pub(crate) fn is_capacity_valid(&self) -> Result<(), super::HandlerBuildError> {
        match self.capacity {
            Some(0) => Err(super::HandlerBuildError::InvalidConfig(
                "capacity must be greater than zero".to_string(),
            )),
            _ => Ok(()),
        }
    }
}
