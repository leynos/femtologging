//! Builder for [`FemtoFileHandler`].
//!
//! Provides a fluent API for configuring a file-based logging handler.
//! Only a subset of options are currently supported; additional
//! parameters such as encoding and mode will be added as the project
//! evolves. Flushing is driven by a `flush_record_interval`
//! measured in records.

use std::num::NonZeroUsize;

use pyo3::prelude::*;

use super::{common::CommonBuilder, file::*, FormatterId, HandlerBuildError, HandlerBuilderTrait};
use crate::{
    formatter::DefaultFormatter,
    macros::{dict_into_py, AsPyDict},
};

/// Builder for constructing [`FemtoFileHandler`] instances.
#[pyclass]
#[derive(Clone, Debug)]
pub struct FileHandlerBuilder {
    path: String,
    common: CommonBuilder,
    flush_record_interval: Option<usize>,
    overflow_policy: OverflowPolicy,
}

impl FileHandlerBuilder {
    /// Create a builder targeting the specified file path.
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            common: CommonBuilder::default(),
            flush_record_interval: None,
            overflow_policy: OverflowPolicy::Drop,
        }
    }

    /// Set the bounded channel capacity.
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.common.capacity = NonZeroUsize::new(capacity);
        self.common.capacity_set = true;
        self
    }

    /// Set the periodic flush interval measured in records. Must be greater
    /// than zero.
    pub fn with_flush_record_interval(mut self, interval: usize) -> Self {
        self.flush_record_interval = Some(interval);
        self
    }

    /// Set the formatter identifier.
    pub fn with_formatter(mut self, formatter_id: impl Into<FormatterId>) -> Self {
        self.common.formatter_id = Some(formatter_id.into());
        self
    }

    /// Set the overflow policy for the handler.
    pub fn with_overflow_policy(mut self, policy: OverflowPolicy) -> Self {
        self.overflow_policy = policy;
        self
    }

    fn is_capacity_valid(&self) -> Result<(), HandlerBuildError> {
        self.common.is_capacity_valid()
    }

    fn is_flush_record_interval_valid(&self) -> Result<(), HandlerBuildError> {
        CommonBuilder::ensure_non_zero(
            "flush_record_interval",
            self.flush_record_interval.map(|v| v as u64),
        )
    }

    fn is_overflow_policy_valid(&self) -> Result<(), HandlerBuildError> {
        match self.overflow_policy {
            OverflowPolicy::Timeout(dur) if dur.is_zero() => Err(HandlerBuildError::InvalidConfig(
                "timeout_ms must be greater than zero".into(),
            )),
            _ => Ok(()),
        }
    }

    fn validate(&self) -> Result<(), HandlerBuildError> {
        self.is_capacity_valid()?;
        self.is_flush_record_interval_valid()?;
        self.is_overflow_policy_valid()?;
        Ok(())
    }
}

impl FileHandlerBuilder {
    /// Populate a Python dictionary with the builder's fields.
    fn fill_pydict(&self, d: &pyo3::Bound<'_, pyo3::types::PyDict>) -> PyResult<()> {
        d.set_item("path", &self.path)?;
        self.common.extend_py_dict(d)?;
        if let Some(flush) = self.flush_record_interval {
            d.set_item("flush_record_interval", flush)?;
        }
        match self.overflow_policy {
            OverflowPolicy::Drop => d.set_item("overflow_policy", "drop")?,
            OverflowPolicy::Block => d.set_item("overflow_policy", "block")?,
            OverflowPolicy::Timeout(dur) => {
                d.set_item("timeout_ms", dur.as_millis() as u64)?;
                d.set_item("overflow_policy", "timeout")?;
            }
        }
        Ok(())
    }
}

impl AsPyDict for FileHandlerBuilder {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let d = pyo3::types::PyDict::new(py);
        self.fill_pydict(&d)?;
        dict_into_py(d, py)
    }
}

#[pymethods]
impl FileHandlerBuilder {
    #[new]
    fn py_new(path: String) -> Self {
        Self::new(path)
    }
    #[pyo3(name = "with_capacity")]
    fn py_with_capacity<'py>(mut slf: PyRefMut<'py, Self>, capacity: usize) -> PyRefMut<'py, Self> {
        slf.common.capacity = NonZeroUsize::new(capacity);
        slf.common.capacity_set = true;
        slf
    }

    #[pyo3(name = "with_flush_record_interval")]
    fn py_with_flush_record_interval<'py>(
        mut slf: PyRefMut<'py, Self>,
        interval: usize,
    ) -> PyRefMut<'py, Self> {
        slf.flush_record_interval = Some(interval);
        slf
    }

    #[pyo3(name = "with_formatter")]
    fn py_with_formatter<'py>(
        mut slf: PyRefMut<'py, Self>,
        formatter_id: String,
    ) -> PyRefMut<'py, Self> {
        slf.common.formatter_id = Some(formatter_id.into());
        slf
    }

    #[pyo3(name = "with_overflow_policy")]
    fn py_with_overflow_policy<'py>(
        mut slf: PyRefMut<'py, Self>,
        policy: &str,
        timeout_ms: Option<u64>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        slf.overflow_policy = if policy.eq_ignore_ascii_case("drop") {
            OverflowPolicy::Drop
        } else if policy.eq_ignore_ascii_case("block") {
            OverflowPolicy::Block
        } else if policy.eq_ignore_ascii_case("timeout") {
            let ms = timeout_ms.ok_or_else(|| {
                PyErr::new::<pyo3::exceptions::PyValueError, _>(
                    "timeout_ms required for timeout policy",
                )
            })?;
            CommonBuilder::ensure_non_zero("timeout_ms", Some(ms)).map_err(PyErr::from)?;
            OverflowPolicy::Timeout(std::time::Duration::from_millis(ms))
        } else {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "invalid overflow policy: {policy}",
            )));
        };
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
        self.validate()?;
        let mut cfg = HandlerConfig::default();
        if let Some(cap) = self.common.capacity {
            cfg.capacity = cap.get();
        }
        if let Some(flush) = self.flush_record_interval {
            cfg.flush_interval = flush;
        }
        cfg.overflow_policy = self.overflow_policy;
        let handler = match self.common.formatter_id.as_ref() {
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
        let dir = tempdir().unwrap();
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
