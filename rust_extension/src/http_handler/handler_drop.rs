//! Drop and debug formatting for [`FemtoHTTPHandler`].
//!
//! `Drop` is where a handler that was never closed explicitly reports the
//! drops it refused and closes its worker, so it belongs with the lifecycle in
//! `handler.rs`. It lives in this child module because the parent file is at
//! the project's per-file size limit and this impl group is a coherent unit to
//! move. As a descendant of the `handler` module it can reach the handler's
//! private fields without widening their visibility.

use super::FemtoHTTPHandler;

#[cfg(not(loom))]
use crate::http_handler::queue::flush_drops;
#[cfg(not(loom))]
use pyo3::Python;

/// Drop the handler, closing it with the GIL released.
///
/// A handler whose worker has already gone has nothing to close, so the check
/// keeps that case out of the GIL's way entirely. When Python can be attached,
/// the close runs detached; during interpreter shutdown, the Rust-only close
/// runs without attaching and remains bounded by the same budget.
///
/// Under `--cfg loom` there is no interpreter to attach to: no model builds one
/// of these handlers, and attaching would initialize Python on a Loom
/// coroutine's small stack.
#[cfg(not(loom))]
impl Drop for FemtoHTTPHandler {
    fn drop(&mut self) {
        // A handler whose worker is gone has nobody left to report the drops
        // refused after an earlier close, and no later warning comes. While a
        // worker is still present, `close()` reports them itself.
        if self.handle.lock().is_none() {
            flush_drops(&self.warner);
            return;
        }
        if Python::try_attach(|py| py.detach(|| self.close())).is_none() {
            self.close();
        }
    }
}

#[cfg(loom)]
impl Drop for FemtoHTTPHandler {
    fn drop(&mut self) {
        self.close();
    }
}

impl std::fmt::Debug for FemtoHTTPHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FemtoHTTPHandler")
            .field("flush_timeout", &self.flush_timeout)
            .finish()
    }
}