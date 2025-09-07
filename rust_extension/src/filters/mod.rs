//! Filtering components for log records.
//!
//! Provides the [`FemtoFilter`] trait along with concrete filter builders for
//! constructing filters.

use std::sync::Arc;

use pyo3::{create_exception, exceptions::PyTypeError, prelude::*};
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

create_exception!(
    _femtologging_rs,
    FilterBuildErrorPy,
    pyo3::exceptions::PyException
);

impl From<FilterBuildError> for PyErr {
    fn from(err: FilterBuildError) -> PyErr {
        FilterBuildErrorPy::new_err(err.to_string())
    }
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
#[non_exhaustive]
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

impl<'py> FromPyObject<'py> for FilterBuilder {
    fn extract_bound(obj: &Bound<'py, PyAny>) -> PyResult<Self> {
        match LevelFilterBuilder::extract_bound(obj) {
            Ok(b) => return Ok(FilterBuilder::Level(b)),
            Err(e) if e.is_instance_of::<PyTypeError>(obj.py()) => {}
            Err(e) => return Err(e),
        }
        match NameFilterBuilder::extract_bound(obj) {
            Ok(b) => return Ok(FilterBuilder::Name(b)),
            Err(e) if e.is_instance_of::<PyTypeError>(obj.py()) => {}
            Err(e) => return Err(e),
        }
        let ty = obj
            .get_type()
            .name()
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "<unknown>".into());
        Err(PyTypeError::new_err(format!(
            "unknown filter builder type (got Python type: {ty})",
        )))
    }
}
