//! Shared buffer utilities for concurrency tests.
//!
//! Provides thread-safe buffer types and helpers for capturing
//! log output in both standard and loom-based scenarios.

pub mod std {
    use std::io::{self, Write};

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

        /// Return a snapshot of the buffer contents.
        #[allow(dead_code)]
        pub fn contents(&self) -> Vec<u8> {
            self.buffer
                .lock()
                .expect("SharedBuf mutex poisoned")
                .clone()
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

    #[allow(dead_code)]
    pub fn read_output(buffer: &Arc<Mutex<Vec<u8>>>) -> String {
        String::from_utf8(buffer.lock().expect("Buffer mutex poisoned").clone())
            .expect("Buffer contains invalid UTF-8")
    }
}

#[allow(dead_code)]
pub mod loom {
    use std::io::{self, Write};

    pub type Arc<T> = loom::sync::Arc<T>;
    pub type Mutex<T> = loom::sync::Mutex<T>;

    /// Wrapper around a loom-backed byte buffer.
    #[derive(Clone)]
    pub struct SharedBuf {
        buffer: Arc<Mutex<Vec<u8>>>,
    }

    impl SharedBuf {
        pub fn new(buffer: Arc<Mutex<Vec<u8>>>) -> Self {
            Self { buffer }
        }

        #[allow(dead_code)]
        pub fn contents(&self) -> Vec<u8> {
            self.buffer
                .lock()
                .expect("SharedBuf mutex poisoned")
                .clone()
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

    #[allow(dead_code)]
    pub fn read_output(buffer: &Arc<Mutex<Vec<u8>>>) -> String {
        String::from_utf8(buffer.lock().expect("Buffer mutex poisoned").clone())
            .expect("Buffer contains invalid UTF-8")
    }
}
