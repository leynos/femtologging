//! Generic JSON round-trip assertion for serde-backed test values.
//!
//! Several schema tests serialized a value to JSON and deserialized it back just
//! to prove the representation survived. Written inline, that duplicated the same
//! two failure branches in every test, so this helper owns them once.
//!
//! The helper panics through an explicit `match` rather than calling `expect`:
//! the lint suite treats `expect` in a non-test helper as an error-handling
//! violation, even though this module is compiled only for tests.

use serde::{Serialize, de::DeserializeOwned};
use std::fmt::Debug;

/// Assert that `value` survives a JSON serialization round trip unchanged.
///
/// The diagnostic names the failing direction, so a schema change that breaks
/// deserialization is distinguishable from one that breaks serialization.
///
/// # Panics
///
/// Panics if `value` cannot be serialized to JSON, if the resulting text cannot
/// be deserialized as `T`, or if the decoded value is not equal to `value`.
///
/// # Examples
///
/// ```rust
/// use crate::test_utils::json_round_trip::assert_json_round_trip;
///
/// let payload = serde_json::json!({"level": "INFO", "message": "started"});
/// assert_json_round_trip(&payload);
/// ```
#[track_caller]
pub fn assert_json_round_trip<T>(value: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + Debug,
{
    let json = match serde_json::to_string(value) {
        Ok(json) => json,
        Err(error) => panic!("value should serialize to JSON: {error}"),
    };
    let decoded: T = match serde_json::from_str(&json) {
        Ok(decoded) => decoded,
        Err(error) => {
            panic!("serialized JSON should deserialize back into the value type: {error}")
        }
    };
    assert_eq!(
        value, &decoded,
        "a JSON round trip should preserve the value"
    );
}
