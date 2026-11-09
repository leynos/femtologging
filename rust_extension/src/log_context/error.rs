//! Public error type for structured logging context operations.

use thiserror::Error;

/// Errors raised when validating or mutating structured logging context.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum LogContextError {
    /// The context stack was popped when no context existed.
    #[error("log context stack is empty")]
    EmptyContextStack,
    /// Too many key-values were supplied for one merged context payload.
    #[error("context has {count} keys; maximum is {max}")]
    TooManyKeys {
        /// Number of keys in the rejected context.
        count: usize,
        /// Maximum number of keys permitted in one context.
        max: usize,
    },
    /// A context key exceeded the byte-length limit.
    #[error("context key '{key}' is {len} bytes; maximum is {max}")]
    KeyTooLong {
        /// The offending key.
        key: String,
        /// UTF-8 byte length of the key.
        len: usize,
        /// Maximum permitted UTF-8 byte length of a key.
        max: usize,
    },
    /// A context key collided with a reserved record or metadata field.
    #[error("context key '{key}' is reserved and cannot be used for structured fields")]
    ReservedKey {
        /// The reserved key that was supplied.
        key: String,
    },
    /// A context value exceeded the byte-length limit.
    #[error("context value for key '{key}' is {len} bytes; maximum is {max}")]
    ValueTooLong {
        /// Key whose value is too long.
        key: String,
        /// UTF-8 byte length of the value.
        len: usize,
        /// Maximum permitted UTF-8 byte length of a value.
        max: usize,
    },
    /// The aggregate key and value byte total exceeded the limit.
    #[error("context payload is {total} bytes; maximum is {max}")]
    TotalBytesExceeded {
        /// Accumulated UTF-8 byte length of the keys and values, up to and including the entry
        /// that crosses `max`. This is not the serialized size.
        total: usize,
        /// Maximum permitted total serialized size, in bytes.
        max: usize,
    },
}
