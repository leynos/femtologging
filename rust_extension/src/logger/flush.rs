//! Synchronous handler flushing for [`super::FemtoLogger`].

use super::FemtoLogger;

impl FemtoLogger {
    /// Flush all handlers attached to this logger.
    ///
    /// This call blocks until the worker acknowledges a flush marker and each
    /// attached handler has flushed. Enqueueing the flush marker and waiting
    /// for its acknowledgement share a two-second timeout. Flushing attached
    /// handlers can take additional time.
    ///
    /// When Python handlers can be attached, callers that currently hold
    /// Python access must detach before calling this method. Otherwise the
    /// worker cannot acquire the GIL to run a Python handler.
    ///
    /// # Returns
    ///
    /// `true` if the worker acknowledges the flush marker within the shared
    /// timeout and every attached handler flush succeeds. `false` if the
    /// marker cannot be enqueued or acknowledged in time, or a handler flush
    /// fails.
    ///
    /// # Examples
    ///
    /// A Rust caller holding Python access detaches during the blocking flush:
    ///
    /// ```
    /// use _femtologging_rs::FemtoLogger;
    /// use pyo3::Python;
    ///
    /// let logger = FemtoLogger::new(String::from("example"))
    ///     .expect("the shared dispatch pool starts");
    /// let flushed = Python::attach(|py| py.detach(|| logger.flush_handlers()));
    /// assert!(flushed);
    /// ```
    pub fn flush_handlers(&self) -> bool {
        #[cfg(all(feature = "python", feature = "test-util"))]
        self.flush_test_hooks.signal_entered();

        self.flush_handlers_blocking()
    }
}
