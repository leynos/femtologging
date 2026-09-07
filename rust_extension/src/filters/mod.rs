//! Filtering components for log records.
//!
//! Provides the [`FemtoFilter`] trait along with concrete filter builders for
//! constructing filters.

use std::{collections::BTreeMap, sync::Arc};

#[cfg(feature = "python")]
use pyo3::prelude::*;
use thiserror::Error;

use crate::log_record::FemtoLogRecord;

/// Outcome returned by the filter pipeline.
#[derive(Debug, Default)]
pub struct FilterDecision {
    /// Whether the record should continue through the pipeline.
    pub(crate) accepted: bool,
    /// Additional metadata fields accepted by the filter.
    pub(crate) enrichment: BTreeMap<String, String>,
}

impl FilterDecision {
    /// Build a decision with no enrichment payload.
    #[must_use]
    pub(crate) const fn accept(accepted: bool) -> Self {
        Self {
            accepted,
            enrichment: BTreeMap::new(),
        }
    }
}

/// Per-record state shared across filters during evaluation.
#[derive(Default)]
pub struct FilterContext {
    #[cfg(feature = "python")]
    pub(crate) python_record_view: Option<Py<PyAny>>,
}

/// Trait implemented by all log filters.
///
/// Filters are `Send + Sync` so they can be shared across threads.
pub trait FemtoFilter: Send + Sync {
    /// Evaluate the filter for `record`, optionally returning enrichment.
    fn decision(&self, record: &mut FemtoLogRecord, context: &mut FilterContext) -> FilterDecision;

    /// Return `true` if `record` should be processed.
    fn should_log(&self, record: &mut FemtoLogRecord) -> bool {
        self.decision(record, &mut FilterContext::default())
            .accepted
    }
}

pub mod level_filter;
pub mod name_filter;
#[cfg(feature = "python")]
pub mod python_callback;
#[cfg(all(test, feature = "python"))]
mod python_callback_tests;
#[cfg(feature = "python")]
mod python_callback_validation;

pub use level_filter::LevelFilterBuilder;
pub use name_filter::NameFilterBuilder;
#[cfg(feature = "python")]
pub use python_callback::PythonCallbackFilterBuilder;

/// Errors that may occur while building a filter.
#[derive(Debug, Error)]
pub enum FilterBuildError {
    /// Invalid user supplied configuration.
    #[error("invalid filter configuration: {0}")]
    InvalidConfig(String),
}

/// Trait implemented by all filter builders.
pub trait FilterBuilderTrait: Send + Sync {
    /// Concrete filter type produced by this builder.
    type Filter: FemtoFilter + 'static;

    /// Build the concrete filter without wrapping it in a trait object.
    ///
    /// # Errors
    ///
    /// Returns [`FilterBuildError`] when the builder configuration is invalid.
    fn build_inner(&self) -> Result<Self::Filter, FilterBuildError>;

    /// Build the filter as a shared trait object.
    ///
    /// # Errors
    ///
    /// Returns [`FilterBuildError`] when the builder configuration is invalid.
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
    /// Build a Python callback filter.
    #[cfg(feature = "python")]
    PythonCallback(PythonCallbackFilterBuilder),
}

impl FilterBuilder {
    pub fn build(&self) -> Result<Arc<dyn FemtoFilter>, FilterBuildError> {
        match self {
            Self::Level(b) => <LevelFilterBuilder as FilterBuilderTrait>::build(b),
            Self::Name(b) => <NameFilterBuilder as FilterBuilderTrait>::build(b),
            #[cfg(feature = "python")]
            Self::PythonCallback(b) => {
                <PythonCallbackFilterBuilder as FilterBuilderTrait>::build(b)
            }
        }
    }
}

impl From<LevelFilterBuilder> for FilterBuilder {
    fn from(value: LevelFilterBuilder) -> Self { Self::Level(value) }
}

impl From<NameFilterBuilder> for FilterBuilder {
    fn from(value: NameFilterBuilder) -> Self { Self::Name(value) }
}

#[cfg(feature = "python")]
impl From<PythonCallbackFilterBuilder> for FilterBuilder {
    fn from(value: PythonCallbackFilterBuilder) -> Self { Self::PythonCallback(value) }
}

#[cfg(feature = "python")]
mod py_helpers {
    //! Python-specific filter helpers grouped to avoid repeated `#[cfg]`
    //! attributes.
    //!
    //! Provides filter-specific Python helpers (exceptions, conversions, dict
    //! adapters).
    #![expect(
        missing_docs,
        reason = "create_exception! generates a public Python exception without a source item \
                  that can carry documentation"
    )]
    use pyo3::{
        Borrowed,
        FromPyObject,
        create_exception,
        exceptions::PyTypeError,
        prelude::{Py, PyAny, PyErr, PyResult, Python},
    };

    use super::{
        FilterBuildError,
        FilterBuilder,
        LevelFilterBuilder,
        NameFilterBuilder,
        PythonCallbackFilterBuilder,
    };
    use crate::{macros::AsPyDict, python::fq_py_type};

    create_exception!(
        _femtologging_rs,
        FilterBuildErrorPy,
        pyo3::exceptions::PyException
    );

    impl From<FilterBuildError> for PyErr {
        fn from(err: FilterBuildError) -> Self { FilterBuildErrorPy::new_err(err.to_string()) }
    }

    impl AsPyDict for FilterBuilder {
        fn as_pydict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
            match self {
                Self::Level(b) => b.as_pydict(py),
                Self::Name(b) => b.as_pydict(py),
                Self::PythonCallback(b) => b.as_pydict(py),
            }
        }
    }

    impl<'a, 'py> FromPyObject<'a, 'py> for FilterBuilder {
        type Error = PyErr;

        fn extract(obj: Borrowed<'a, 'py, PyAny>) -> Result<Self, Self::Error> {
            if let Ok(builder) = obj.extract::<LevelFilterBuilder>() {
                return Ok(Self::Level(builder));
            }

            if let Ok(builder) = obj.extract::<NameFilterBuilder>() {
                return Ok(Self::Name(builder));
            }

            if let Ok(builder) = obj.extract::<PythonCallbackFilterBuilder>() {
                return Ok(Self::PythonCallback(builder));
            }

            if let Ok(builder) = PythonCallbackFilterBuilder::from_callback_obj(obj.to_owned()) {
                return Ok(Self::PythonCallback(builder));
            }

            let fq = fq_py_type(&obj.to_owned());
            Err(PyTypeError::new_err(format!(
                "builder must be LevelFilterBuilder, NameFilterBuilder, or a Python callback \
                 filter (got Python type: {fq})",
            )))
        }
    }
}

#[cfg(feature = "python")]
pub use py_helpers::FilterBuildErrorPy;
