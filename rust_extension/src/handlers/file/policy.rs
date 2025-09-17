//! Overflow policy parsing helpers for the file handler.
//!
//! Provides shared parsing logic for Python-facing APIs so behaviour
//! remains consistent between constructors and builder methods.

use std::time::Duration;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use super::config::OverflowPolicy;

#[derive(Debug)]
struct PyOverflowPolicyConfig {
    policy: String,
    timeout_ms: Option<u64>,
}

impl PyOverflowPolicyConfig {
    fn new(policy: String, timeout_ms: Option<u64>) -> Self {
        Self { policy, timeout_ms }
    }

    fn normalise(policy: &str) -> String {
        policy.trim().to_ascii_lowercase()
    }

    fn invalid_message(policy: &str) -> String {
        let valid = ["drop", "block", "timeout:N"].join(", ");
        format!("invalid overflow policy '{policy}'. Valid options are: {valid}")
    }

    fn from_policy_string(policy: &str) -> PyResult<Self> {
        let normalized = Self::normalise(policy);
        match normalized.as_str() {
            "drop" | "block" => Ok(Self::new(normalized, None)),
            "timeout" => Err(PyValueError::new_err(
                "timeout requires a positive integer N, use 'timeout:N'",
            )),
            _ => {
                if let Some(rest) = normalized.strip_prefix("timeout:") {
                    let ms: i64 = rest.trim().parse().map_err(|_| {
                        PyValueError::new_err(
                            "timeout must be a positive integer (N in 'timeout:N')",
                        )
                    })?;
                    if ms <= 0 {
                        return Err(PyValueError::new_err("timeout must be greater than zero"));
                    }
                    Ok(Self::new("timeout".to_string(), Some(ms as u64)))
                } else {
                    Err(PyValueError::new_err(Self::invalid_message(&normalized)))
                }
            }
        }
    }

    #[cfg(feature = "python")]
    fn from_policy_and_timeout(policy: &str, timeout_ms: Option<u64>) -> PyResult<Self> {
        let normalized = Self::normalise(policy);
        match normalized.as_str() {
            "drop" | "block" => Ok(Self::new(normalized, None)),
            "timeout" => {
                let ms = timeout_ms.ok_or_else(|| {
                    PyValueError::new_err("timeout_ms required for timeout policy")
                })?;
                if ms == 0 {
                    return Err(PyValueError::new_err(
                        "timeout_ms must be greater than zero",
                    ));
                }
                Ok(Self::new(normalized, Some(ms)))
            }
            _ => Err(PyValueError::new_err(Self::invalid_message(&normalized))),
        }
    }
}

impl TryFrom<&PyOverflowPolicyConfig> for OverflowPolicy {
    type Error = PyErr;

    fn try_from(config: &PyOverflowPolicyConfig) -> PyResult<Self> {
        match config.policy.as_str() {
            "drop" => Ok(OverflowPolicy::Drop),
            "block" => Ok(OverflowPolicy::Block),
            "timeout" => {
                let ms = config.timeout_ms.ok_or_else(|| {
                    PyValueError::new_err("timeout_ms required for timeout policy")
                })?;
                Ok(OverflowPolicy::Timeout(Duration::from_millis(ms)))
            }
            other => Err(PyValueError::new_err(
                PyOverflowPolicyConfig::invalid_message(other),
            )),
        }
    }
}

pub(crate) fn parse_policy_string(policy: &str) -> PyResult<OverflowPolicy> {
    let config = PyOverflowPolicyConfig::from_policy_string(policy)?;
    OverflowPolicy::try_from(&config)
}

#[cfg(feature = "python")]
pub(crate) fn parse_policy_with_timeout(
    policy: &str,
    timeout_ms: Option<u64>,
) -> PyResult<OverflowPolicy> {
    let config = PyOverflowPolicyConfig::from_policy_and_timeout(policy, timeout_ms)?;
    OverflowPolicy::try_from(&config)
}

#[cfg(all(test, feature = "python"))]
mod tests {
    use super::*;
    use pyo3::Python;

    fn assert_value_error_message(err: PyErr, expected: &str) {
        Python::with_gil(|py| {
            assert!(err.is_instance_of::<PyValueError>(py));
            assert_eq!(err.value(py).to_string(), expected);
        });
    }

    #[test]
    fn parse_policy_string_accepts_drop_whitespace() {
        assert_eq!(parse_policy_string(" drop ").unwrap(), OverflowPolicy::Drop);
    }

    #[test]
    fn parse_policy_string_accepts_block_case_insensitive() {
        assert_eq!(parse_policy_string("BLOCK").unwrap(), OverflowPolicy::Block);
    }

    #[test]
    fn parse_policy_string_parses_timeout_values() {
        assert_eq!(
            parse_policy_string("timeout:250").unwrap(),
            OverflowPolicy::Timeout(Duration::from_millis(250))
        );
    }

    #[test]
    fn parse_policy_string_rejects_missing_timeout_value() {
        assert_value_error_message(
            parse_policy_string("timeout").unwrap_err(),
            "timeout requires a positive integer N, use 'timeout:N'",
        );
    }

    #[test]
    fn parse_policy_string_rejects_non_numeric_timeout_value() {
        assert_value_error_message(
            parse_policy_string("timeout:abc").unwrap_err(),
            "timeout must be a positive integer (N in 'timeout:N')",
        );
    }

    #[test]
    fn parse_policy_string_rejects_zero_timeout_value() {
        assert_value_error_message(
            parse_policy_string("timeout:0").unwrap_err(),
            "timeout must be greater than zero",
        );
    }

    #[test]
    fn parse_policy_string_rejects_unknown_policy() {
        assert_value_error_message(
            parse_policy_string("unknown").unwrap_err(),
            "invalid overflow policy 'unknown'. Valid options are: drop, block, timeout:N",
        );
    }

    #[test]
    fn parse_policy_with_timeout_supports_drop() {
        assert_eq!(
            parse_policy_with_timeout("drop", None).unwrap(),
            OverflowPolicy::Drop
        );
    }

    #[test]
    fn parse_policy_with_timeout_requires_timeout_value() {
        assert_value_error_message(
            parse_policy_with_timeout("timeout", None).unwrap_err(),
            "timeout_ms required for timeout policy",
        );
    }

    #[test]
    fn parse_policy_with_timeout_rejects_zero_timeout_value() {
        assert_value_error_message(
            parse_policy_with_timeout("timeout", Some(0)).unwrap_err(),
            "timeout_ms must be greater than zero",
        );
    }

    #[test]
    fn parse_policy_with_timeout_accepts_timeout_value() {
        assert_eq!(
            parse_policy_with_timeout("timeout", Some(500)).unwrap(),
            OverflowPolicy::Timeout(Duration::from_millis(500))
        );
    }
}
