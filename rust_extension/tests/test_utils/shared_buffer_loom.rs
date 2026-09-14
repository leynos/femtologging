//! Loom-backed output buffers used only by model-checking test modules.

use std::io::{self, ErrorKind, Seek, SeekFrom, Write};

/// Shared ownership primitive instrumented by Loom for scheduling exploration.
pub type Arc<T> = loom::sync::Arc<T>;
/// Mutual-exclusion primitive instrumented by Loom for scheduling exploration.
pub type Mutex<T> = loom::sync::Mutex<T>;

/// Writer façade around a Loom-synchronised byte buffer.
#[derive(Clone)]
pub struct SharedBuf {
    /// The output bytes whose synchronisation Loom explores.
    buffer: Arc<Mutex<Vec<u8>>>,
}

impl SharedBuf {
    /// Wrap a caller-owned Loom buffer for use as a handler writer.
    pub fn new(buffer: Arc<Mutex<Vec<u8>>>) -> Self {
        Self { buffer }
    }
}

impl Default for SharedBuf {
    fn default() -> Self {
        Self {
            buffer: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer
            .lock()
            .expect("SharedBuf mutex poisoned")
            .write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.buffer
            .lock()
            .expect("SharedBuf mutex poisoned")
            .flush()
    }
}

impl Seek for SharedBuf {
    fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(
            ErrorKind::Unsupported,
            "seek unsupported for SharedBuf",
        ))
    }
}

/// Decode a Loom buffer after a model has completed its writes.
pub fn read_output(buffer: &Arc<Mutex<Vec<u8>>>) -> String {
    String::from_utf8(buffer.lock().expect("Buffer mutex poisoned").clone())
        .expect("Buffer contains invalid UTF-8")
}
