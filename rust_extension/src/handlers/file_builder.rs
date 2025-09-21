//! Builder for [`FemtoFileHandler`].
//!
//! Provides a fluent API for configuring a file-based logging handler.
//! Only a subset of options are currently supported; additional
//! parameters such as encoding and mode will be added as the project
//! evolves. Flushing is driven by a `flush_record_interval`
//! measured in records.

#[cfg(feature = "python")]
use pyo3::prelude::*;

use super::{
    common::FileLikeBuilderState, file::*, FormatterId, HandlerBuildError, HandlerBuilderTrait,
};
use crate::formatter::DefaultFormatter;

use crate::handlers::builder_macros::{builder_method_rust, file_like_builder_methods};
#[cfg(feature = "python")]
use crate::macros::{dict_into_py, AsPyDict};

/// Builder for constructing [`FemtoFileHandler`] instances.
#[cfg_attr(feature = "python", pyclass)]
#[derive(Clone, Debug)]
pub struct FileHandlerBuilder {
    path: String,
    state: FileLikeBuilderState,
}

impl FileHandlerBuilder {
    /// Create a builder targeting the specified file path.
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            state: FileLikeBuilderState::default(),
        }
    }

    file_like_builder_methods!(builder_method_rust);

    #[cfg(feature = "python")]
    fn apply_capacity(&mut self, capacity: usize) {
        self.state.set_capacity(capacity);
    }

    #[cfg(feature = "python")]
    fn apply_flush_record_interval(&mut self, interval: usize) {
        self.state.set_flush_record_interval(interval);
    }

    #[cfg(feature = "python")]
    fn apply_formatter(&mut self, formatter_id: FormatterId) {
        self.state.set_formatter(formatter_id);
    }

    /// Set the overflow policy for the handler.
    pub fn with_overflow_policy(mut self, policy: OverflowPolicy) -> Self {
        self.state.set_overflow_policy(policy);
        self
    }
}

#[cfg(feature = "python")]
impl FileHandlerBuilder {
    /// Populate a Python dictionary with the builder's fields.
    fn fill_pydict(&self, d: &pyo3::Bound<'_, pyo3::types::PyDict>) -> PyResult<()> {
        d.set_item("path", &self.path)?;
        self.state.extend_py_dict(d)?;
        Ok(())
    }
}

#[cfg(feature = "python")]
impl AsPyDict for FileHandlerBuilder {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let d = pyo3::types::PyDict::new(py);
        self.fill_pydict(&d)?;
        dict_into_py(d, py)
    }
}

#[cfg(feature = "python")]
#[pymethods]
impl FileHandlerBuilder {
    #[new]
    fn py_new(path: String) -> Self {
        Self::new(path)
    }
    #[pyo3(name = "with_capacity")]
    fn py_with_capacity<'py>(mut slf: PyRefMut<'py, Self>, capacity: usize) -> PyRefMut<'py, Self> {
        slf.apply_capacity(capacity);
        slf
    }

    #[pyo3(name = "with_flush_record_interval")]
    fn py_with_flush_record_interval<'py>(
        mut slf: PyRefMut<'py, Self>,
        interval: usize,
    ) -> PyRefMut<'py, Self> {
        slf.apply_flush_record_interval(interval);
        slf
    }

    #[pyo3(name = "with_formatter")]
    fn py_with_formatter<'py>(
        mut slf: PyRefMut<'py, Self>,
        formatter_id: String,
    ) -> PyRefMut<'py, Self> {
        slf.apply_formatter(FormatterId::from(formatter_id));
        slf
    }

    #[pyo3(name = "with_overflow_policy")]
    fn py_with_overflow_policy<'py>(
        mut slf: PyRefMut<'py, Self>,
        policy: &str,
        timeout_ms: Option<u64>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let policy_value = policy::parse_policy_with_timeout(policy, timeout_ms)?;
        slf.state.set_overflow_policy(policy_value);
        Ok(slf)
    }

    /// Return a dictionary describing the builder configuration.
    fn as_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        self.as_pydict(py)
    }

    /// Build the handler, raising ``HandlerConfigError`` or ``HandlerIOError`` on
    /// failure.
    fn build(&self) -> PyResult<FemtoFileHandler> {
        <Self as HandlerBuilderTrait>::build_inner(self).map_err(PyErr::from)
    }
}

impl HandlerBuilderTrait for FileHandlerBuilder {
    type Handler = FemtoFileHandler;

    /// Build a [`FemtoFileHandler`].
    ///
    /// `DEFAULT_CHANNEL_CAPACITY` (1024) when `with_capacity` is not called.
    fn build_inner(&self) -> Result<Self::Handler, HandlerBuildError> {
        self.state.validate()?;
        let cfg = self.state.handler_config();
        let handler = match self.state.formatter_id() {
            Some(FormatterId::Default) | None => {
                FemtoFileHandler::with_capacity_flush_policy(&self.path, DefaultFormatter, cfg)?
            }
            Some(FormatterId::Custom(other)) => {
                return Err(HandlerBuildError::InvalidConfig(format!(
                    "unknown formatter id: {other}",
                )))
            }
        };
        Ok(handler)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::assert_build_err;
    use super::*;
    use rstest::rstest;
    use tempfile::tempdir;

    #[rstest]
    fn build_file_handler() {
        let dir = tempdir().expect("tempdir must create a temporary directory");
        let path = dir.path().join("test.log");
        let builder = FileHandlerBuilder::new(path.to_string_lossy())
            .with_capacity(16)
            .with_flush_record_interval(1);
        let handler = builder
            .build_inner()
            .expect("build_inner must succeed for a valid file builder");
        handler.flush();
    }

    #[rstest]
    fn reject_zero_capacity() {
        let builder = FileHandlerBuilder::new("log.txt").with_capacity(0);
        assert_build_err(&builder, "build_inner must fail for zero capacity");
    }

    #[rstest]
    fn reject_zero_flush_record_interval() {
        let builder = FileHandlerBuilder::new("log.txt").with_flush_record_interval(0);
        assert_build_err(
            &builder,
            "build_inner must fail for zero flush record interval",
        );
    }

    #[rstest]
    fn reject_zero_overflow_timeout() {
        let builder = FileHandlerBuilder::new("log.txt")
            .with_overflow_policy(OverflowPolicy::Timeout(std::time::Duration::from_millis(0)));
        assert_build_err(&builder, "build_inner must fail for zero timeout_ms");
    }
}
