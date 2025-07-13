//! Shared buffer utilities for concurrency tests.
//!
//! Provides thread-safe buffer types and helpers for capturing
//! log output in both standard and loom-based scenarios.

use crate::{Arc, Mutex};
use std::io::{self, Write};

#[derive(Clone)]
pub struct SharedBuf(Arc<Mutex<Vec<u8>>>);

impl SharedBuf {
    /// Create a new, empty `SharedBuf`.
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(Vec::new())))
    }

    /// Access the inner `Arc<Mutex<Vec<u8>>>`.
    pub fn inner(&self) -> &Arc<Mutex<Vec<u8>>> {
        &self.0
    }
}

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("Failed to lock SharedBuf for writing")
            .write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0
            .lock()
            .expect("Failed to lock SharedBuf for flushing")
            .flush()
    }
}

fn read_output(buffer: &Arc<Mutex<Vec<u8>>>) -> String {
    String::from_utf8(
        buffer
            .lock()
            .expect("Failed to lock buffer for reading")
            .clone(),
    )
    .expect("Buffer did not contain valid UTF-8")
}
