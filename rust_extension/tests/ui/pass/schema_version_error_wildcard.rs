//! Compile-pass coverage for matching `SchemaVersionError`.
//!
//! The enum is `#[non_exhaustive]`, so downstream crates must keep a wildcard
//! arm after naming every current variant. This fixture proves that the
//! required arm type-checks against the public re-export and that the nested
//! and limit variants are reachable from outside the crate.

use _femtologging_rs::{ExceptionPayload, SchemaVersionError, SchemaVersioned};

/// Classify a validation error using the wildcard arm the enum requires.
fn classify(error: &SchemaVersionError) -> &'static str {
    match error {
        SchemaVersionError::VersionTooNew { .. } => "too new",
        SchemaVersionError::VersionTooOld { .. } => "too old",
        SchemaVersionError::Nested { .. } => "nested",
        SchemaVersionError::TreeTooDeep { .. } => "too deep",
        SchemaVersionError::TreeTooLarge { .. } => "too large",
        // Required because `SchemaVersionError` is `#[non_exhaustive]`; this
        // arm also keeps later variants compiling.
        _ => "unsupported",
    }
}

fn main() {
    let payload = ExceptionPayload::new("Error", "message");
    let _ = payload.validate_version().map_err(|error| classify(&error));
    let _ = classify as fn(&SchemaVersionError) -> &'static str;
}