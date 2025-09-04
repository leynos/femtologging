//! Builders for common log filters.
//!
//! Provides builder types and traits for constructing filters.

use std::sync::Arc;

use pyo3::prelude::*;
use thiserror::Error;

use crate::{filter::FemtoFilter, macros::AsPyDict};

pub mod level_filter;
pub mod name_filter;

pub use level_filter::LevelFilterBuilder;
pub use name_filter::NameFilterBuilder;

/// Errors that may occur while building a filter.
#[derive(Debug, Error)]
pub enum FilterBuildError {
    /// Invalid user supplied configuration.
    #[error("invalid filter configuration: {0}")]
    InvalidConfig(String),
}

/// Trait implemented by all filter builders.
pub trait FilterBuilderTrait: Send + Sync {
    type Filter: FemtoFilter + 'static;

    fn build_inner(&self) -> Result<Self::Filter, FilterBuildError>;

    fn build(&self) -> Result<Arc<dyn FemtoFilter>, FilterBuildError> {
        Ok(Arc::new(self.build_inner()?))
    }
}

/// Concrete filter builder variants.
#[derive(Clone, Debug)]
pub enum FilterBuilder {
    /// Build a [`LevelFilter`].
    Level(LevelFilterBuilder),
    /// Build a [`NameFilter`].
    Name(NameFilterBuilder),
}

impl FilterBuilder {
    pub fn build(&self) -> Result<Arc<dyn FemtoFilter>, FilterBuildError> {
        match self {
            Self::Level(b) => <LevelFilterBuilder as FilterBuilderTrait>::build(b),
            Self::Name(b) => <NameFilterBuilder as FilterBuilderTrait>::build(b),
        }
    }
}

impl From<LevelFilterBuilder> for FilterBuilder {
    fn from(value: LevelFilterBuilder) -> Self {
        Self::Level(value)
    }
}

impl From<NameFilterBuilder> for FilterBuilder {
    fn from(value: NameFilterBuilder) -> Self {
        Self::Name(value)
    }
}

impl AsPyDict for FilterBuilder {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<PyObject> {
        match self {
            Self::Level(b) => b.as_pydict(py),
            Self::Name(b) => b.as_pydict(py),
        }
    }
}
