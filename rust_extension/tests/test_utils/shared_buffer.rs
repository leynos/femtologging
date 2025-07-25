//! Shared buffer utilities for concurrency tests.
//!
//! Provides thread-safe buffer types and helpers for capturing log output in
//! both standard and loom-based scenarios.
//!
//! Use [`SharedBuf::new`] to construct instances while keeping the internal
//! `Arc<Mutex<Vec<u8>>>` hidden, requiring callers to lock before access.

macro_rules! shared_buf_mod {
    ($name:ident, $arc:path, $mutex:path) => {
        #[expect(dead_code)]
        pub mod $name {
            use std::io::{self, Write};

            pub use $arc as Arc;
            pub use $mutex as Mutex;

            /// Thread-safe buffer wrapper used in tests.
            ///
            /// The inner `Arc<Mutex<Vec<u8>>>` is private to enforce locking.
            #[derive(Clone)]
            pub struct SharedBuf {
                buffer: Arc<Mutex<Vec<u8>>>,
            }

            impl SharedBuf {
                /// Creates a new `SharedBuf` wrapping the provided buffer.
                pub fn new(buffer: Arc<Mutex<Vec<u8>>>) -> Self {
                    Self { buffer }
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

            /// Returns the current contents of the buffer as a UTF-8 string.
            ///
            /// The `buffer` parameter is an `Arc`-wrapped `Mutex` guarding a
            /// `Vec<u8>`. The mutex is locked before the bytes are cloned and
            /// converted into UTF-8.
            ///
            /// # Panics
            ///
            /// Panics if locking the mutex fails or if the bytes are not valid
            /// UTF-8.
            pub fn read_output(buffer: &Arc<Mutex<Vec<u8>>>) -> String {
                String::from_utf8(buffer.lock().expect("Buffer mutex poisoned").clone())
                    .expect("Buffer contains invalid UTF-8")
            }
        }
    };
}

shared_buf_mod!(std, std::sync::Arc, std::sync::Mutex);
shared_buf_mod!(loom, loom::sync::Arc, loom::sync::Mutex);
