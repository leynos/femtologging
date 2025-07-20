//! Shared buffer utilities for concurrency tests.
//!
//! Provides thread-safe buffer types and helpers for capturing
//! log output in both standard and loom-based scenarios.

pub mod std {
    use std::io::{self, Write};

    pub type Arc<T> = std::sync::Arc<T>;
    pub type Mutex<T> = std::sync::Mutex<T>;

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
    pub struct SharedBuf(pub Arc<Mutex<Vec<u8>>>);

    impl Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("SharedBuf mutex poisoned").write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.0.lock().expect("SharedBuf mutex poisoned").flush()
        }
    }

    #[allow(dead_code)]
    pub fn read_output(buffer: &Arc<Mutex<Vec<u8>>>) -> String {
        String::from_utf8(buffer.lock().expect("Buffer mutex poisoned").clone())
            .expect("Buffer contains invalid UTF-8")
    }
}
