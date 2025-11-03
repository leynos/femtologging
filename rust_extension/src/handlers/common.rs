//! Shared builder options.
//!
//! Stores fields common to multiple handler builders.

use std::{
    fmt,
    num::{NonZeroU64, NonZeroUsize},
};

#[cfg(feature = "python")]
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

#[cfg(feature = "python")]
use pyo3::{
    class::basic::CompareOp,
    exceptions::{PyNotImplementedError, PyTypeError, PyValueError},
    prelude::*,
    types::{PyDict, PyString},
    Bound, IntoPyObjectExt,
};

use super::{
    file::{HandlerConfig, OverflowPolicy},
    FormatterId, HandlerBuildError,
};
use crate::formatter::{FemtoFormatter, SharedFormatter};

/// Formatter configuration stored by handler builders.
#[derive(Clone)]
pub enum FormatterConfig {
    /// Formatter referenced by identifier.
    Id(FormatterId),
    /// Formatter provided as an instance.
    Instance(SharedFormatter),
}

impl fmt::Debug for FormatterConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Id(id) => f.debug_tuple("Id").field(id).finish(),
            Self::Instance(_) => f.write_str("Instance(<formatter>)"),
        }
    }
}

/// Convert inputs into [`FormatterConfig`] values for builder consumption.
pub trait IntoFormatterConfig {
    /// Convert `self` into a [`FormatterConfig`].
    fn into_formatter_config(self) -> FormatterConfig;
}

impl<F> IntoFormatterConfig for F
where
    F: FemtoFormatter + 'static,
{
    fn into_formatter_config(self) -> FormatterConfig {
        FormatterConfig::Instance(SharedFormatter::new(self))
    }
}

impl IntoFormatterConfig for SharedFormatter {
    fn into_formatter_config(self) -> FormatterConfig {
        FormatterConfig::Instance(self)
    }
}

impl IntoFormatterConfig for FormatterId {
    fn into_formatter_config(self) -> FormatterConfig {
        FormatterConfig::Id(self)
    }
}

impl IntoFormatterConfig for String {
    fn into_formatter_config(self) -> FormatterConfig {
        FormatterId::from(self).into_formatter_config()
    }
}

impl IntoFormatterConfig for &str {
    fn into_formatter_config(self) -> FormatterConfig {
        FormatterId::from(self).into_formatter_config()
    }
}

#[cfg(feature = "python")]
#[pyclass(name = "OverflowPolicy")]
#[derive(Clone)]
pub struct PyOverflowPolicy {
    pub(crate) inner: OverflowPolicy,
}

#[cfg(feature = "python")]
#[pymethods]
impl PyOverflowPolicy {
    #[staticmethod]
    fn drop() -> Self {
        Self {
            inner: OverflowPolicy::Drop,
        }
    }

    #[staticmethod]
    fn block() -> Self {
        Self {
            inner: OverflowPolicy::Block,
        }
    }

    #[staticmethod]
    fn timeout(timeout_ms: u64) -> PyResult<Self> {
        if timeout_ms == 0 {
            return Err(PyValueError::new_err("timeout must be greater than zero"));
        }
        Ok(Self {
            inner: OverflowPolicy::Timeout(std::time::Duration::from_millis(timeout_ms)),
        })
    }

    fn __str__(&self) -> String {
        self.__repr__()
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            OverflowPolicy::Drop => "OverflowPolicy.drop()".to_string(),
            OverflowPolicy::Block => "OverflowPolicy.block()".to_string(),
            OverflowPolicy::Timeout(duration) => {
                format!("OverflowPolicy.timeout({})", duration.as_millis())
            }
        }
    }

    fn __richcmp__<'py>(&'py self, other: &Bound<'py, PyAny>, op: CompareOp) -> PyResult<bool> {
        match op {
            CompareOp::Eq => {
                if let Ok(other_policy) = other.extract::<PyRef<'py, PyOverflowPolicy>>() {
                    Ok(self.inner == other_policy.inner)
                } else {
                    Ok(false)
                }
            }
            CompareOp::Ne => {
                if let Ok(other_policy) = other.extract::<PyRef<'py, PyOverflowPolicy>>() {
                    Ok(self.inner != other_policy.inner)
                } else {
                    Ok(true)
                }
            }
            _ => Err(PyNotImplementedError::new_err("ordering not supported")),
        }
    }

    fn __hash__(&self) -> PyResult<isize> {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);

        let mut value = hasher.finish();
        while isize::try_from(value).is_err() {
            value >>= 1;
        }

        Ok(isize::try_from(value).expect("hash reduction must fit in isize"))
    }
}

#[derive(Clone, Debug, Default)]
pub struct CommonBuilder {
    pub(crate) capacity: Option<NonZeroUsize>,
    pub(crate) capacity_set: bool,
    pub(crate) flush_timeout_ms: Option<NonZeroU64>,
    pub(crate) formatter: Option<FormatterConfig>,
}

impl CommonBuilder {
    pub(crate) const DEFAULT_FLUSH_TIMEOUT_MS: u64 = 1_000;

    /// Update the bounded channel capacity in place.
    ///
    /// A zero capacity is recorded for validation but does not update the
    /// stored [`NonZeroUsize`]. Callers rely on [`is_capacity_valid`] to surface
    /// the configuration error when `build` is invoked.
    pub(crate) fn set_capacity(&mut self, capacity: usize) {
        if capacity == 0 {
            self.capacity = None;
            self.capacity_set = true;
            return;
        }

        self.capacity = Some(
            NonZeroUsize::new(capacity)
                .expect("NonZeroUsize::new must succeed for non-zero capacity"),
        );
        self.capacity_set = true;
    }

    pub(crate) fn set_formatter<F>(&mut self, formatter: F)
    where
        F: IntoFormatterConfig,
    {
        self.formatter = Some(formatter.into_formatter_config());
    }

    #[cfg(feature = "python")]
    pub(crate) fn set_formatter_from_py(&mut self, formatter: &Bound<'_, PyAny>) -> PyResult<()> {
        match formatter.downcast::<PyString>() {
            Ok(py_str) => {
                self.set_formatter(py_str.to_str()?.to_owned());
                Ok(())
            }
            Err(downcast_err) => match crate::formatter::python::formatter_from_py(formatter) {
                Ok(instance) => {
                    // The extracted formatter is already wrapped in a shared trait
                    // object; storing it directly avoids double `Arc` wrapping via the
                    // blanket `IntoFormatterConfig` implementation.
                    self.formatter = Some(FormatterConfig::Instance(instance));
                    Ok(())
                }
                Err(instance_err) => {
                    let py = formatter.py();

                    let string_err: PyErr = downcast_err.into();
                    let string_context =
                        PyTypeError::new_err("formatter string identifier extraction failed");
                    string_context.set_cause(py, Some(string_err));

                    if let Some(existing_cause) = instance_err.cause(py) {
                        let bound_cause = existing_cause.clone_ref(py).into_bound_py_any(py)?;
                        let callable_err = PyErr::from_value(bound_cause);
                        callable_err.set_cause(py, Some(string_context));
                        instance_err.set_cause(py, Some(callable_err));
                    } else {
                        instance_err.set_cause(py, Some(string_context));
                    }

                    Err(instance_err)
                }
            },
        }
    }

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

    /// Extend a Python dictionary with common builder fields.
    #[cfg(feature = "python")]
    pub(crate) fn extend_py_dict(&self, d: &Bound<'_, PyDict>) -> PyResult<()> {
        if let Some(cap) = self.capacity {
            d.set_item("capacity", cap.get())?;
        }
        if let Some(ms) = self.flush_timeout_ms {
            d.set_item("flush_timeout_ms", ms.get())?;
        }
        if let Some(fmt) = &self.formatter {
            match fmt {
                FormatterConfig::Id(fid) => {
                    d.set_item("formatter_kind", "id")?;
                    d.set_item("formatter_id", fid.as_str())?;
                }
                FormatterConfig::Instance(_) => {
                    d.set_item("formatter_kind", "instance")?;
                    d.set_item("formatter", "instance")?;
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub(crate) struct FileLikeBuilderState {
    pub(crate) common: CommonBuilder,
    pub(crate) flush_record_interval: Option<usize>,
    pub(crate) overflow_policy: OverflowPolicy,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_capacity_stores_non_zero_value() {
        let mut builder = CommonBuilder::default();
        builder.set_capacity(32);

        let stored = builder
            .capacity
            .expect("set_capacity must store a NonZeroUsize for non-zero input");
        assert_eq!(stored.get(), 32);
        assert!(
            builder.capacity_set,
            "set_capacity must mark capacity as configured"
        );
    }

    #[test]
    fn set_capacity_zero_is_reported_invalid() {
        let mut builder = CommonBuilder::default();
        builder.set_capacity(0);

        assert!(
            builder.capacity.is_none(),
            "zero capacity must not store a value"
        );
        assert!(
            builder.capacity_set,
            "zero capacity must record that configuration was attempted"
        );
        let err = builder
            .is_capacity_valid()
            .expect_err("zero capacity must be rejected during validation");
        assert!(matches!(
            err,
            HandlerBuildError::InvalidConfig(message) if message == "capacity must be greater than zero"
        ));
    }
}

impl Default for FileLikeBuilderState {
    fn default() -> Self {
        Self::new()
    }
}

impl FileLikeBuilderState {
    /// Create a new builder state with default queue settings.
    pub(crate) fn new() -> Self {
        Self {
            common: CommonBuilder::default(),
            flush_record_interval: None,
            overflow_policy: OverflowPolicy::Drop,
        }
    }

    /// Update the bounded channel capacity in place.
    pub(crate) fn set_capacity(&mut self, capacity: usize) {
        self.common.set_capacity(capacity);
    }

    /// Update the flush interval in place.
    pub(crate) fn set_flush_record_interval(&mut self, interval: usize) {
        self.flush_record_interval = Some(interval);
    }

    /// Update the formatter identifier in place.
    pub(crate) fn set_formatter<F>(&mut self, formatter: F)
    where
        F: IntoFormatterConfig,
    {
        self.common.set_formatter(formatter);
    }

    /// Update the overflow policy in place.
    pub(crate) fn set_overflow_policy(&mut self, policy: OverflowPolicy) {
        self.overflow_policy = policy;
    }

    #[cfg(feature = "python")]
    pub(crate) fn set_formatter_from_py(&mut self, formatter: &Bound<'_, PyAny>) -> PyResult<()> {
        self.common.set_formatter_from_py(formatter)
    }

    /// Validate queue-related settings shared between file-based builders.
    pub(crate) fn validate(&self) -> Result<(), HandlerBuildError> {
        self.common.is_capacity_valid()?;
        CommonBuilder::ensure_non_zero(
            "flush_record_interval",
            self.flush_record_interval.map(|value| value as u64),
        )?;
        if let OverflowPolicy::Timeout(duration) = self.overflow_policy {
            if duration.is_zero() {
                return Err(HandlerBuildError::InvalidConfig(
                    "timeout_ms must be greater than zero".into(),
                ));
            }
        }
        Ok(())
    }

    /// Produce a [`HandlerConfig`] populated with the configured values.
    pub(crate) fn handler_config(&self) -> HandlerConfig {
        let mut cfg = HandlerConfig::default();
        if let Some(capacity) = self.common.capacity {
            cfg.capacity = capacity.get();
        }
        if let Some(interval) = self.flush_record_interval {
            cfg.flush_interval = interval;
        }
        cfg.overflow_policy = self.overflow_policy;
        cfg
    }

    /// Expose the configured formatter identifier, if any.
    pub(crate) fn formatter(&self) -> Option<&FormatterConfig> {
        self.common.formatter.as_ref()
    }

    /// Extend a Python dictionary with shared file builder fields.
    #[cfg(feature = "python")]
    pub(crate) fn extend_py_dict(&self, d: &Bound<'_, PyDict>) -> PyResult<()> {
        self.common.extend_py_dict(d)?;
        if let Some(flush) = self.flush_record_interval {
            d.set_item("flush_record_interval", flush)?;
        }
        match self.overflow_policy {
            OverflowPolicy::Drop => d.set_item("overflow_policy", "drop")?,
            OverflowPolicy::Block => d.set_item("overflow_policy", "block")?,
            OverflowPolicy::Timeout(duration) => {
                d.set_item("timeout_ms", duration.as_millis() as u64)?;
                d.set_item("overflow_policy", "timeout")?;
            }
        }
        Ok(())
    }
}
