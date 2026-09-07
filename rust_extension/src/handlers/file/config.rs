//! Configuration structures for [`super::FemtoFileHandler`].
//!
//! This module defines the various configuration types used when constructing
//! and testing file handlers. The public API exposes [`HandlerConfig`] for Rust
//! callers. Overflow handling and channel capacity are also defined here so
//! they can be shared between the handler implementation and worker thread
//! logic.

use std::{
    sync::{Arc, Barrier},
    time::Duration,
};

/// Default bounded channel capacity for `FemtoFileHandler`.
pub const DEFAULT_CHANNEL_CAPACITY: usize = 1024;

/// Determines how `FemtoFileHandler` reacts when its queue is full.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OverflowPolicy {
    /// Drop new records, preserving existing ones.
    Drop,
    /// Block the caller until space becomes available.
    Block,
    /// Block up to the specified duration before giving up.
    Timeout(Duration),
}

/// Configuration options for constructing a [`super::FemtoFileHandler`].
#[derive(Clone, Copy)]
pub struct HandlerConfig {
    /// Bounded queue size for records waiting to be written.
    pub capacity: usize,
    /// How often the worker thread flushes the writer.
    pub flush_interval: usize,
    /// Policy to apply when the queue is full.
    pub overflow_policy: OverflowPolicy,
}

impl Default for HandlerConfig {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_CHANNEL_CAPACITY,
            flush_interval: 1,
            overflow_policy: OverflowPolicy::Drop,
        }
    }
}

/// Configuration for `with_writer_for_test` when constructing handlers in tests.
pub struct TestConfig<W, F> {
    /// Writer supplied to the test handler.
    pub writer: W,
    /// Formatter supplied to the test handler.
    pub formatter: F,
    /// Queue capacity used by the test handler.
    pub capacity: usize,
    /// Flush interval used by the test handler.
    pub flush_interval: usize,
    /// Overflow policy used by the test handler.
    pub overflow_policy: OverflowPolicy,
    /// Optional barrier used to synchronize worker startup in tests.
    pub start_barrier: Option<Arc<Barrier>>,
}

impl<W, F> TestConfig<W, F> {
    /// Create a test configuration with the default queue settings.
    pub const fn new(writer: W, formatter: F) -> Self {
        Self {
            writer,
            formatter,
            capacity: DEFAULT_CHANNEL_CAPACITY,
            flush_interval: 1,
            overflow_policy: OverflowPolicy::Drop,
            start_barrier: None,
        }
    }
}
