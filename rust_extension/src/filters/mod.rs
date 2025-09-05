//! Filtering components for log records.
//!
//! Provides the [`FemtoFilter`] trait along with builders and a registry for
//! constructing filters.

use std::sync::Arc;

use pyo3::prelude::*;
use thiserror::Error;

use crate::{log_record::FemtoLogRecord, macros::AsPyDict};

/// Trait implemented by all log filters.
///
/// Filters are `Send + Sync` so they can be shared across threads.
pub trait FemtoFilter: Send + Sync {
    /// Return `true` if `record` should be processed.
    fn should_log(&self, record: &FemtoLogRecord) -> bool;
}

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

/// Function type used to extract a [`FilterBuilder`] from Python objects.
///
/// Builders register their extractor via [`inventory`], allowing new builders to
/// integrate without modifying central dispatch code.
pub type ExtractFilterBuilder = fn(&Bound<'_, PyAny>) -> PyResult<Option<FilterBuilder>>;

pub struct FilterExtractor(pub ExtractFilterBuilder);

inventory::collect!(FilterExtractor);

/// Try to extract a [`FilterBuilder`] from a Python object by consulting the
/// registered extractors.
pub fn extract_filter_builder(obj: &Bound<'_, PyAny>) -> PyResult<FilterBuilder> {
    for extractor in inventory::iter::<FilterExtractor> {
        if let Some(builder) = (extractor.0)(obj)? {
            return Ok(builder);
        }
    }
    Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
        "unknown filter builder",
    ))
}
