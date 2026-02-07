//! Filtering components for log records.
//!
//! Provides the [`FemtoFilter`] trait along with concrete filter builders for
//! constructing filters.

use std::sync::Arc;

use thiserror::Error;

use crate::log_record::FemtoLogRecord;

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

#[cfg(feature = "python")]
mod py_helpers {
    //! Python-specific filter helpers grouped to avoid repeated `#[cfg]`
    //! attributes.
    //!
    //! Provides filter-specific Python helpers (exceptions, conversions, dict
    //! adapters).
    use super::*;
    use crate::macros::AsPyDict;
    use crate::python::fq_py_type;
    use pyo3::{Borrowed, create_exception, exceptions::PyTypeError, prelude::*};

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

    impl AsPyDict for FilterBuilder {
        fn as_pydict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
            match self {
                Self::Level(b) => b.as_pydict(py),
                Self::Name(b) => b.as_pydict(py),
            }
        }
    }

    impl<'a, 'py> FromPyObject<'a, 'py> for FilterBuilder {
        type Error = PyErr;

        fn extract(obj: Borrowed<'a, 'py, PyAny>) -> Result<Self, Self::Error> {
            if let Ok(builder) = obj.extract::<LevelFilterBuilder>() {
                return Ok(FilterBuilder::Level(builder));
            }

            if let Ok(builder) = obj.extract::<NameFilterBuilder>() {
                return Ok(FilterBuilder::Name(builder));
            }

            let fq = fq_py_type(&obj.to_owned());
            Err(PyTypeError::new_err(format!(
                "builder must be LevelFilterBuilder or NameFilterBuilder (got Python type: {fq})",
            )))
        }
    }
}

#[cfg(feature = "python")]
pub use py_helpers::FilterBuildErrorPy;
