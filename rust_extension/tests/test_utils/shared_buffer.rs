//! Shared buffer utilities for concurrency tests.
//!
//! Provides thread-safe buffer types and helpers for capturing
//! log output in both standard and loom-based scenarios.

use crate::{Arc, Mutex};
use std::io::{self, Write};

#[derive(Clone)]
pub struct SharedBuf(pub Arc<Mutex<Vec<u8>>>);

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("SharedBuf mutex poisoned").write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().expect("SharedBuf mutex poisoned").flush()
    }
}

/// Return the captured output as a `String`.
///
/// This clones the entire buffer for simplicity, which is fine for tests
/// but could be expensive with large data sets.
pub fn read_output(buffer: &Arc<Mutex<Vec<u8>>>) -> String {
    String::from_utf8(buffer.lock().expect("Buffer mutex poisoned").clone())
        .expect("Buffer contains invalid UTF-8")
}
