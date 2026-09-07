//! Typed parsing for the Python `filter_frames` call boundary.

use pyo3::{
    Bound,
    exceptions::PyTypeError,
    prelude::*,
    types::{PyDict, PyTuple},
};

/// Parsed Python arguments for `filter_frames`.
pub(super) struct FilterFramesRequest<'py> {
    /// Payload dictionary supplied as the required argument.
    pub(super) payload: Bound<'py, PyDict>,
    /// Filename patterns that should be excluded.
    pub(super) exclude_filenames: Option<Vec<String>>,
    /// Function-name patterns that should be excluded.
    pub(super) exclude_functions: Option<Vec<String>>,
    /// Maximum number of recent frames to retain.
    pub(super) max_depth: Option<usize>,
    /// Whether standard logging frames should be removed.
    pub(super) exclude_logging: bool,
}

/// Raw positional and keyword values received by the Python function.
struct FilterFramesArguments<'a, 'py> {
    positional: &'a Bound<'py, PyTuple>,
    keywords: Option<&'a Bound<'py, PyDict>>,
}

impl<'a, 'py> FilterFramesArguments<'a, 'py> {
    const NAMES: [&'static str; 5] = [
        "payload",
        "exclude_filenames",
        "exclude_functions",
        "max_depth",
        "exclude_logging",
    ];

    fn new(
        positional: &'a Bound<'py, PyTuple>,
        keywords: Option<&'a Bound<'py, PyDict>>,
    ) -> PyResult<Self> {
        if positional.len() > 1 {
            return Err(PyTypeError::new_err(format!(
                "filter_frames() takes 1 positional argument but {} were given",
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
                "filter_frames() got an unexpected keyword argument '{key_name}'",
            )))
        }
    }

    fn name(index: usize) -> PyResult<&'static str> {
        Self::NAMES.get(index).copied().ok_or_else(|| {
            PyTypeError::new_err(format!(
                "filter_frames() internal argument index {index} is invalid"
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
                "filter_frames() got multiple values for argument '{name}'",
            )));
        }
        Ok(positional_value.or(keyword_value))
    }
}

impl<'py> FilterFramesRequest<'py> {
    /// Parse the legacy Python positional and keyword call shape into typed options.
    pub(super) fn from_python(
        args: &Bound<'py, PyTuple>,
        kwargs: Option<&Bound<'py, PyDict>>,
    ) -> PyResult<Self> {
        let arguments = FilterFramesArguments::new(args, kwargs)?;
        let payload_value = arguments.value(0)?.ok_or_else(|| {
            PyTypeError::new_err(
                "filter_frames() missing 1 required positional argument: 'payload'",
            )
        })?;
        let payload = payload_value.cast::<PyDict>()?.clone();
        let exclude_filenames = arguments
            .value(1)?
            .map(|value| value.extract())
            .transpose()?;
        let exclude_functions = arguments
            .value(2)?
            .map(|value| value.extract())
            .transpose()?;
        let max_depth = arguments
            .value(3)?
            .map(|value| value.extract())
            .transpose()?;
        let exclude_logging = arguments
            .value(4)?
            .map_or(Ok(false), |value| value.extract())?;

        Ok(Self {
            payload,
            exclude_filenames,
            exclude_functions,
            max_depth,
            exclude_logging,
        })
    }
}
