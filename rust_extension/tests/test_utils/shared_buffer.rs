//! Shared buffer utilities for concurrency tests.
//!
//! Provides thread-safe buffer types and helpers for capturing
//! log output in both standard and loom-based scenarios.

pub mod std {
    //! Shared buffer backed by std synchronization primitives.

    use std::io::{self, ErrorKind, Seek, SeekFrom, Write};

    pub type Arc<T> = std::sync::Arc<T>;
    pub type Mutex<T> = std::sync::Mutex<T>;

    /// Thread-safe wrapper around a byte buffer used by stream handlers.
    ///
    /// The inner `Arc<Mutex<Vec<u8>>>` is kept private so tests can't
    /// accidentally bypass the `Write` implementation or mutate the buffer
    /// without locking.
    #[derive(Clone)]
    pub struct SharedBuf {
        buffer: Arc<Mutex<Vec<u8>>>,
    }

    impl SharedBuf {
        /// Create a new `SharedBuf` backed by the given shared buffer.
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

    pub fn read_output(buffer: &Arc<Mutex<Vec<u8>>>) -> String {
        String::from_utf8(buffer.lock().expect("Buffer mutex poisoned").clone())
            .expect("Buffer contains invalid UTF-8")
    }
}
