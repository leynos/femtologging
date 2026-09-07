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
        pub const fn new(buffer: Arc<Mutex<Vec<u8>>>) -> Self { Self { buffer } }
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
            let mut buffer = match self.buffer.lock() {
                Ok(buffer) => buffer,
                Err(poisoned) => poisoned.into_inner(),
            };
            buffer.write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            let mut buffer = match self.buffer.lock() {
                Ok(buffer) => buffer,
                Err(poisoned) => poisoned.into_inner(),
            };
            buffer.flush()
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

    /// Return the shared buffer as UTF-8 text.
    ///
    /// # Errors
    ///
    /// Returns [`std::string::FromUtf8Error`] when the buffer contains invalid
    /// UTF-8 bytes.
    pub fn read_output(buffer: &Arc<Mutex<Vec<u8>>>) -> Result<String, std::string::FromUtf8Error> {
        let bytes = match buffer.lock() {
            Ok(locked_buffer) => locked_buffer.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        String::from_utf8(bytes)
    }
}

#[cfg(loom)]
pub mod loom {
    //! Shared buffer backed by loom synchronization primitives for model checking.

    use std::io::{self, ErrorKind, Seek, SeekFrom, Write};

    pub type Arc<T> = loom::sync::Arc<T>;
    pub type Mutex<T> = loom::sync::Mutex<T>;

    /// Wrapper around a loom-backed byte buffer.
    #[derive(Clone)]
    pub struct SharedBuf {
        buffer: Arc<Mutex<Vec<u8>>>,
    }

    impl SharedBuf {
        pub const fn new(buffer: Arc<Mutex<Vec<u8>>>) -> Self { Self { buffer } }
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
            let mut buffer = match self.buffer.lock() {
                Ok(buffer) => buffer,
                Err(poisoned) => poisoned.into_inner(),
            };
            buffer.write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            let mut buffer = match self.buffer.lock() {
                Ok(buffer) => buffer,
                Err(poisoned) => poisoned.into_inner(),
            };
            buffer.flush()
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

    /// Return the shared buffer as UTF-8 text.
    ///
    /// # Errors
    ///
    /// Returns [`std::string::FromUtf8Error`] when the buffer contains invalid
    /// UTF-8 bytes.
    pub fn read_output(buffer: &Arc<Mutex<Vec<u8>>>) -> Result<String, std::string::FromUtf8Error> {
        let bytes = match buffer.lock() {
            Ok(locked_buffer) => locked_buffer.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        String::from_utf8(bytes)
    }
}
