//! Shared buffer utilities for concurrency tests.
//!
//! Provides thread-safe buffer types and helpers for capturing
//! log output in both standard and loom-based scenarios. Construct
//! `SharedBuf` using [`SharedBuf::new`] to keep the underlying
//! `Arc<Mutex<Vec<u8>>>` encapsulated.

pub mod std {
    use std::io::{self, Write};

    pub type Arc<T> = std::sync::Arc<T>;
    pub type Mutex<T> = std::sync::Mutex<T>;

    /// Thread-safe buffer wrapper used in tests.
    ///
    /// The inner `Arc<Mutex<Vec<u8>>>` is private to prevent
    /// direct mutation without locking. Use `new` to create and
    /// `buffer` to access the underlying data.
    /// Thread-safe buffer wrapper used in `loom` tests.
    ///
    /// Like the `std` variant, the inner buffer is private to
    /// enforce locking discipline. Use `new` and `buffer()` to
    /// create and access it.
    #[derive(Clone)]
    pub struct SharedBuf {
        buffer: Arc<Mutex<Vec<u8>>>,
    }

    impl SharedBuf {
        pub fn new(buffer: Arc<Mutex<Vec<u8>>>) -> Self {
            Self { buffer }
        }

        #[allow(dead_code)]
        pub fn buffer(&self) -> &Arc<Mutex<Vec<u8>>> {
            &self.buffer
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

    #[derive(Clone)]
    pub struct SharedBuf {
        buffer: Arc<Mutex<Vec<u8>>>,
    }

    impl SharedBuf {
        pub fn new(buffer: Arc<Mutex<Vec<u8>>>) -> Self {
            Self { buffer }
        }

        #[allow(dead_code)]
        pub fn buffer(&self) -> &Arc<Mutex<Vec<u8>>> {
            &self.buffer
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
