//! Validation budget for bounded structured-context admission.

use super::{
    ContextValidationError, MAX_CONTEXT_KEYS, MAX_KEY_BYTES, MAX_TOTAL_BYTES, MAX_VALUE_BYTES,
    is_reserved_structured_key,
};

/// Tracks retained key count and aggregate UTF-8 size as entries are validated.
#[derive(Default)]
pub(crate) struct ContextBudget {
    /// Number of distinct keys already included in the candidate map.
    unique_keys: usize,
    /// UTF-8 bytes for retained keys and their current values.
    total_bytes: usize,
}

impl ContextBudget {
    /// Validate a candidate entry before the caller retains converted strings.
    pub(crate) fn validate_next(
        &mut self,
        key: &str,
        value: &str,
        previous_value: Option<&str>,
    ) -> Result<(), ContextValidationError> {
        validate_context_key(key)?;
        let is_new_key = previous_value.is_none();
        self.validate_key_capacity(is_new_key)?;
        validate_context_value(key, value)?;
        self.commit_entry_size(key, value, previous_value)
    }

    /// Reject a new distinct key once the map holds its maximum.
    fn validate_key_capacity(&self, is_new_key: bool) -> Result<(), ContextValidationError> {
        if is_new_key && self.unique_keys == MAX_CONTEXT_KEYS {
            return Err(ContextValidationError::TooManyKeys {
                count: MAX_CONTEXT_KEYS + 1,
                max: MAX_CONTEXT_KEYS,
            });
        }
        Ok(())
    }

    /// Reject an entry that would push the retained payload past its byte limit.
    fn commit_entry_size(
        &mut self,
        key: &str,
        value: &str,
        previous_value: Option<&str>,
    ) -> Result<(), ContextValidationError> {
        let is_new_key = previous_value.is_none();
        let next_total_bytes = self.total_bytes - previous_value.map_or(0, str::len)
            + value.len()
            + if is_new_key { key.len() } else { 0 };
        if next_total_bytes > MAX_TOTAL_BYTES {
            return Err(ContextValidationError::TotalBytesExceeded {
                total: next_total_bytes,
                max: MAX_TOTAL_BYTES,
            });
        }
        if is_new_key {
            self.unique_keys += 1;
        }
        self.total_bytes = next_total_bytes;
        Ok(())
    }
}

/// Reject a key that exceeds the per-key limit or shadows a built-in field.
fn validate_context_key(key: &str) -> Result<(), ContextValidationError> {
    let key_len = key.len();
    if key_len > MAX_KEY_BYTES {
        return Err(ContextValidationError::KeyTooLong {
            len: key_len,
            max: MAX_KEY_BYTES,
        });
    }
    if is_reserved_structured_key(key) {
        return Err(ContextValidationError::ReservedKey {
            key: key.to_owned(),
        });
    }
    Ok(())
}

/// Reject a value that exceeds the per-value byte limit.
fn validate_context_value(key: &str, value: &str) -> Result<(), ContextValidationError> {
    let value_len = value.len();
    if value_len > MAX_VALUE_BYTES {
        return Err(ContextValidationError::ValueTooLong {
            key: key.to_owned(),
            len: value_len,
            max: MAX_VALUE_BYTES,
        });
    }
    Ok(())
}
