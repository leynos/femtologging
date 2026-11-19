//! Supported schema versions, their errors, and the `SchemaVersioned` trait.
//!
//! [`validate_schema_version`] checks a single version against the inclusive
//! range [`MIN_EXCEPTION_SCHEMA_VERSION`] through
//! [`EXCEPTION_SCHEMA_VERSION`]. [`SchemaVersioned`] exposes that check as
//! `validate_version`; a payload that nests independently versioned payloads
//! overrides the method so the whole tree is validated, as
//! [`ExceptionPayload`](super::ExceptionPayload) does.
//!
//! [`SchemaVersionError`] keeps the original root-level variants unchanged and
//! adds path-bearing failures for a nested mismatch and for the bounded walk.

use thiserror::Error;

use super::{EXCEPTION_SCHEMA_VERSION, MIN_EXCEPTION_SCHEMA_VERSION};


/// Errors related to exception schema version validation.
///
/// The enum is `#[non_exhaustive]`: nested-payload and bounded-tree failures
/// were added after the first release, so consumers must keep a wildcard arm
/// when matching.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum SchemaVersionError {
    /// The payload schema version is newer than supported.
    #[error(
        "unsupported exception schema version: found {found}, \
         maximum supported is {max_supported}"
    )]
    VersionTooNew {
        /// The schema version found in the payload.
        found: u16,
        /// The maximum schema version supported by this library.
        max_supported: u16,
    },
    /// The payload schema version is older than supported.
    #[error(
        "unsupported exception schema version: found {found}, \
         minimum supported is {min_supported}"
    )]
    VersionTooOld {
        /// The schema version found in the payload.
        found: u16,
        /// The minimum schema version supported by this library.
        min_supported: u16,
    },
    /// A nested payload carried an unsupported schema version.
    ///
    /// `path` locates the offending payload using dot-separated `cause`,
    /// `context`, and `exceptions[N]` segments, for example
    /// `cause.exceptions[3].context`. `source` is always
    /// [`SchemaVersionError::VersionTooNew`] or
    /// [`SchemaVersionError::VersionTooOld`].
    #[error("unsupported exception schema version at `{path}`: {source}")]
    Nested {
        /// Dot-separated path from the root payload to the failing payload.
        path: String,
        /// The underlying version mismatch.
        #[source]
        source: Box<SchemaVersionError>,
    },
    /// The payload tree nested deeper than validation traverses.
    #[error(
        "exception payload tree exceeds the maximum depth at `{path}`: \
         maximum depth is {max_depth}"
    )]
    TreeTooDeep {
        /// Path from the root to the first payload past the depth limit.
        path: String,
        /// Maximum depth that validation traverses.
        max_depth: usize,
    },
    /// The payload tree held more nodes than validation visits.
    #[error("exception payload tree has more than {max_nodes} nodes")]
    TreeTooLarge {
        /// Maximum number of nodes that validation visits.
        max_nodes: usize,
    },
}

/// Validate that a schema version is supported.
///
/// Returns `Ok(())` if the version is in the supported range
/// ([`MIN_EXCEPTION_SCHEMA_VERSION`] through [`EXCEPTION_SCHEMA_VERSION`]),
/// or an error if the version is unsupported.
///
/// # Errors
///
/// Returns [`SchemaVersionError::VersionTooNew`] if the version exceeds
/// [`EXCEPTION_SCHEMA_VERSION`], or [`SchemaVersionError::VersionTooOld`]
/// if the version is below [`MIN_EXCEPTION_SCHEMA_VERSION`].
///
/// # Examples
///
/// ```rust
/// use _femtologging_rs::exception_schema::{
///     validate_schema_version, EXCEPTION_SCHEMA_VERSION, MIN_EXCEPTION_SCHEMA_VERSION,
/// };
///
/// assert!(validate_schema_version(MIN_EXCEPTION_SCHEMA_VERSION).is_ok());
/// assert!(validate_schema_version(EXCEPTION_SCHEMA_VERSION).is_ok());
/// assert!(validate_schema_version(EXCEPTION_SCHEMA_VERSION + 1).is_err());
/// ```
pub fn validate_schema_version(version: u16) -> Result<(), SchemaVersionError> {
    if version > EXCEPTION_SCHEMA_VERSION {
        return Err(SchemaVersionError::VersionTooNew {
            found: version,
            max_supported: EXCEPTION_SCHEMA_VERSION,
        });
    }
    if version < MIN_EXCEPTION_SCHEMA_VERSION {
        return Err(SchemaVersionError::VersionTooOld {
            found: version,
            min_supported: MIN_EXCEPTION_SCHEMA_VERSION,
        });
    }
    Ok(())
}

/// Trait for types that carry a schema version.
///
/// Implementing this trait provides a blanket `validate_version` method
/// via the extension trait pattern.
///
/// An implementation that holds independently versioned nested payloads must
/// override [`SchemaVersioned::validate_version`] so that validation covers
/// the whole tree. The default implementation checks only
/// [`SchemaVersioned::schema_version`]. [`validate_schema_version`] remains
/// available for checking a single value explicitly.
pub trait SchemaVersioned {
    /// Returns the schema version of this payload.
    fn schema_version(&self) -> u16;

    /// Validate that this payload's schema version is supported.
    ///
    /// Implementations that nest independently versioned payloads override
    /// this to validate every payload they reach.
    ///
    /// # Errors
    ///
    /// Returns [`SchemaVersionError::VersionTooNew`] if the version exceeds
    /// the maximum supported, or [`SchemaVersionError::VersionTooOld`] if
    /// below the minimum supported.
    fn validate_version(&self) -> Result<(), SchemaVersionError> {
        validate_schema_version(self.schema_version())
    }
}
// weave: run 'weave explain rust_extension/src/exception_schema/versioning.rs' for per-hunk detail, 'weave check' to verify your resolution
