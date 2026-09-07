//! Structured parsing for the `TimedHandlerOptions` Python constructor.

use chrono::NaiveTime;
use pyo3::{
    Bound,
    exceptions::PyTypeError,
    prelude::*,
    types::{PyDict, PyTuple},
};

use super::{TimedHandlerOptions, extract_naive_time};
use crate::handlers::file::DEFAULT_CHANNEL_CAPACITY;

/// Raw positional and keyword values received by the Python constructor.
struct ConstructorArguments<'arguments, 'py> {
    positional: &'arguments Bound<'py, PyTuple>,
    keywords: Option<&'arguments Bound<'py, PyDict>>,
}

impl<'arguments, 'py> ConstructorArguments<'arguments, 'py> {
    const NAMES: [&'static str; 8] = [
        "capacity",
        "flush_interval",
        "policy",
        "when",
        "interval",
        "backup_count",
        "utc",
        "at_time",
    ];

    fn new(
        positional: &'arguments Bound<'py, PyTuple>,
        keywords: Option<&'arguments Bound<'py, PyDict>>,
    ) -> PyResult<Self> {
        if positional.len() > Self::NAMES.len() {
            return Err(PyTypeError::new_err(format!(
                "TimedHandlerOptions expected at most {} positional arguments, got {}",
                Self::NAMES.len(),
                positional.len(),
            )));
        }
        if let Some(keyword_values) = keywords {
            for (key, _) in keyword_values.iter() {
                Self::validate_keyword_name(&key)?;
            }
        }
        Ok(Self {
            positional,
            keywords,
        })
    }

    fn validate_keyword_name(key: &Bound<'py, PyAny>) -> PyResult<()> {
        let key_name = key.extract::<&str>()?;
        if Self::NAMES.contains(&key_name) {
            Ok(())
        } else {
            Err(PyTypeError::new_err(format!(
                "TimedHandlerOptions got an unexpected keyword argument {key_name:?}",
            )))
        }
    }

    fn name(index: usize) -> PyResult<&'static str> {
        Self::NAMES.get(index).copied().ok_or_else(|| {
            PyTypeError::new_err(format!(
                "TimedHandlerOptions internal argument position {index} is out of range",
            ))
        })
    }

    fn value(&self, index: usize) -> PyResult<Option<Bound<'py, PyAny>>> {
        let name = Self::name(index)?;
        let positional_value = (index < self.positional.len())
            .then(|| self.positional.get_item(index))
            .transpose()?;
        let keyword_value = self
            .keywords
            .map(|keywords| keywords.get_item(name))
            .transpose()?
            .flatten();
        if positional_value.is_some() && keyword_value.is_some() {
            return Err(PyTypeError::new_err(format!(
                "TimedHandlerOptions got multiple values for argument {name:?}",
            )));
        }
        Ok(positional_value.or(keyword_value))
    }

    fn extract_or_default<T>(&self, index: usize, default: T) -> PyResult<T>
    where
        T: FromPyObjectOwned<'py>,
    {
        self.value(index)?
            .map_or_else(|| Ok(default), |value| value.extract().map_err(Into::into))
    }

    fn optional_at_time(&self) -> PyResult<Option<NaiveTime>> {
        match self.value(7)? {
            Some(value) if value.is_none() => Ok(None),
            Some(value) => extract_naive_time(&value).map(Some),
            None => Ok(None),
        }
    }
}

/// Fully typed constructor request that preserves the Python option contract.
pub(super) struct TimedHandlerOptionsRequest {
    capacity: usize,
    flush_interval: isize,
    policy: String,
    when: String,
    interval: u32,
    backup_count: usize,
    utc: bool,
    at_time: Option<NaiveTime>,
}

impl TimedHandlerOptionsRequest {
    /// Parse legacy positional and keyword options without widening the Rust API.
    pub(super) fn from_python<'py>(
        args: &Bound<'py, PyTuple>,
        kwargs: Option<&Bound<'py, PyDict>>,
    ) -> PyResult<Self> {
        let arguments = ConstructorArguments::new(args, kwargs)?;
        let capacity: usize = arguments.extract_or_default(0, DEFAULT_CHANNEL_CAPACITY)?;
        let flush_interval: isize = arguments.extract_or_default(1, 1)?;
        let policy: String = arguments.extract_or_default(2, "drop".to_owned())?;
        let when: String = arguments.extract_or_default(3, "H".to_owned())?;
        let interval: u32 = arguments.extract_or_default(4, 1)?;
        let backup_count: usize = arguments.extract_or_default(5, 0)?;
        let utc: bool = arguments.extract_or_default(6, false)?;
        let at_time = arguments.optional_at_time()?;
        Ok(Self {
            capacity,
            flush_interval,
            policy,
            when,
            interval,
            backup_count,
            utc,
            at_time,
        })
    }
}

impl From<TimedHandlerOptionsRequest> for TimedHandlerOptions {
    fn from(request: TimedHandlerOptionsRequest) -> Self {
        Self {
            capacity: request.capacity,
            flush_interval: request.flush_interval,
            policy: request.policy,
            when: request.when,
            interval: request.interval,
            backup_count: request.backup_count,
            utc: request.utc,
            at_time: request.at_time,
        }
    }
}
