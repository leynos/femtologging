//! Python methods exposed by the `FemtoLogger` PyO3 class.
//!
//! This `#[pymethods]` block adds the constructor `new`, the `level` and
//! `propagate` accessors with `set_level`, handler and filter management
//! through `py_add_handler`, `py_remove_handler`, `py_clear_handlers` and
//! `py_clear_filters`, the `get_dropped` and `get_disabled_drops` counters,
//! and `py_flush_handlers`, which releases the GIL while the barrier runs.
//! Fallible entry points return `PyResult`; `log` lives in
//! `convenience_methods` and flush diagnostics in `flush_diagnostics_api`.
//! A second, non-`#[pymethods]` block holds `flush_handlers_detached`, the
//! detachment seam that keeps `pyo3` out of the barrier module: the Python
//! method and the log-compat manager sweep both reach it with the interpreter
//! token they already hold.

use std::sync::{Arc, atomic::Ordering};

use pyo3::prelude::*;
use pyo3::{Py, PyAny};

use crate::{handler::FemtoHandlerTrait, level::FemtoLevel};

use super::flush::FlushFailureStage;
#[cfg(all(feature = "python", any(test, feature = "test-util")))]
use super::test_identity::AttachmentIdentity;
use super::{FemtoLogger, PyHandler, validate_handler};

#[pymethods]
impl FemtoLogger {
    /// Create a new logger with the given name.
    ///
    /// Names must satisfy the same rules as
    /// [`crate::manager::get_logger`]: at most
    /// [`MAX_LOGGER_NAME_BYTES`](crate::manager::MAX_LOGGER_NAME_BYTES) bytes,
    /// at most
    /// [`MAX_LOGGER_NAME_SEGMENTS`](crate::manager::MAX_LOGGER_NAME_SEGMENTS)
    /// dot-separated segments, no leading or trailing dot, no empty segment,
    /// and no line-breaking character: any control character (C0, DEL or C1),
    /// or the Unicode line and paragraph separators. A logger built this way
    /// is not registered in the global manager, so it does not count against
    /// the registry limit.
    ///
    /// Raises
    /// ------
    /// ValueError
    ///     If the name breaks the rules above.
    /// RuntimeError
    ///     If the process cannot start the shared dispatch pool.
    #[new]
    #[pyo3(text_signature = "(name)")]
    pub fn new(name: String) -> PyResult<Self> {
        crate::manager::validate_logger_name(&name)?;
        Ok(Self::with_parent(name, None)?)
    }

    /// Update the logger's minimum level.
    ///
    /// `level` accepts "TRACE", "DEBUG", "INFO", "WARN", "ERROR", or
    /// "CRITICAL". The update is thread-safe because the level is stored in a
    /// shared `AtomicU8`. With `log-compat`, it also refreshes the facade's
    /// conservative process-wide maximum level.
    #[pyo3(text_signature = "(self, level)")]
    pub fn set_level(&self, level: FemtoLevel) {
        #[cfg(feature = "log-compat")]
        crate::log_compat::update_logger_level(&self.level, level);
        #[cfg(not(feature = "log-compat"))]
        self.level.store(u8::from(level), Ordering::Relaxed);
    }

    /// Return the logger's current minimum level as a string.
    ///
    /// This method is thread-safe; the level is stored in an `AtomicU8` and
    /// read with `Ordering::Relaxed`.
    #[getter]
    pub fn level(&self) -> String {
        self.load_level().to_string()
    }

    /// Return whether this logger propagates records to its parent (affecting parent-propagation behaviour).
    #[getter]
    pub fn propagate(&self) -> bool {
        self.propagate.load(Ordering::SeqCst)
    }

    /// Return whether configuration has disabled this logger.
    ///
    /// A disabled logger rejects records in its own producer path, so it emits
    /// nothing locally and nothing reaches its ancestors. The flag is set by
    /// `ConfigBuilder::build_and_init` when `disable_existing_loggers` omits
    /// this logger, and cleared when a later configuration names it
    /// explicitly.
    #[getter]
    pub fn disabled(&self) -> bool {
        self.is_disabled()
    }

    /// Set whether this logger propagates records to its parent, controlling parent-propagation behaviour.
    #[pyo3(text_signature = "(self, flag)")]
    pub fn set_propagate(&self, flag: bool) {
        self.propagate.store(flag, Ordering::SeqCst);
    }

    /// Attach a handler implemented in Python or Rust.
    #[pyo3(name = "add_handler", text_signature = "(self, handler)")]
    pub fn py_add_handler(&self, handler: Py<PyAny>) -> PyResult<()> {
        Python::attach(|py| {
            let obj = handler.bind(py);
            validate_handler(obj)?;
            let py_handler = PyHandler::new(py, handler);
            self.add_handler(Arc::new(py_handler) as Arc<dyn FemtoHandlerTrait>)?;
            Ok(())
        })
    }

    /// Remove a handler that was previously attached via `add_handler`.
    ///
    /// Returns `True` when one matching attachment was removed. Selection runs
    /// outside every guard and the removal re-checks the selection under the
    /// write guard, so a re-entrant caller can remove a selected attachment
    /// first. One scan therefore yields every match, and every one of them is
    /// tried; a re-entrant removal costs one target rather than the whole
    /// call's remaining budget. A further scan is made only when a re-entrant
    /// caller removed everything the previous scan found.
    #[pyo3(name = "remove_handler", text_signature = "(self, handler)")]
    pub fn py_remove_handler(&self, handler: Py<PyAny>) -> bool {
        Python::attach(|py| {
            for _ in 0..MAX_REMOVAL_SCANS {
                let targets = self.select_py_handlers(py, &handler);
                if targets.is_empty() {
                    return false;
                }
                if targets
                    .into_iter()
                    .any(|target| self.remove_selected_handler(&target))
                {
                    return true;
                }
            }
            false
        })
    }

    /// Remove all attached handlers.
    #[pyo3(name = "clear_handlers", text_signature = "(self)")]
    pub fn py_clear_handlers(&self) {
        self.clear_handlers();
    }

    /// Remove all attached filters.
    #[pyo3(name = "clear_filters", text_signature = "(self)")]
    pub fn py_clear_filters(&self) {
        self.clear_filters();
    }

    /// Return the number of records dropped due to a full or shutting-down queue.
    ///
    /// Useful for tests and monitoring dashboards.
    #[pyo3(text_signature = "(self)")]
    pub fn get_dropped(&self) -> u64 {
        self.dropped_records.load(Ordering::Relaxed)
    }

    /// Return how many records were refused because the logger is disabled.
    ///
    /// The counter rises for records aimed at a logger that
    /// `disable_existing_loggers` has disabled, so a deployment can tell
    /// configuration-disabled traffic apart from queue pressure. A later
    /// configuration that names the logger re-enables it and stops the rise.
    #[pyo3(text_signature = "(self)")]
    pub fn get_disabled_drops(&self) -> u64 {
        self.disabled_dropped_records.load(Ordering::Relaxed)
    }

    /// Flush all handlers attached to this logger, releasing the GIL while the
    /// worker and handlers finish.
    ///
    /// Resolves dispatch state, queues a marker for a non-empty handler list,
    /// then snapshots handlers before allowing the worker to run callbacks.
    /// With no handlers, it checks only this logger's in-flight work. One
    /// absolute 2 second deadline covers all stages; built-ins retain shorter
    /// timeouts. Python calls release the GIL.
    /// Legacy synchronous handlers fail closed; deadline-aware overrides must
    /// honour the deadline because code cannot be preempted. Violations may
    /// keep the shared worker busy after ``False`` but cannot hold this caller.
    /// Changes completed before the handler snapshot are reflected in this
    /// barrier; removals after it do not cancel the queued flush.
    ///
    /// Returns
    /// -------
    /// bool
    ///     ``True`` on success; ``False`` when completion is unconfirmed or a handler fails.
    ///
    /// Examples
    /// --------
    /// >>> logger.flush_handlers()
    /// True
    #[pyo3(name = "flush_handlers", text_signature = "(self)")]
    fn py_flush_handlers(&self, py: Python<'_>) -> bool {
        self.flush_handlers_detached(py)
    }
}

/// Test-only identity hooks for attachment assertions.
///
/// These methods live in their own implementation block because they exist
/// only when the extension is built with `test-util`; the production
/// `#[pymethods]` block above must not gain test seams. They return opaque
/// tokens rather than native addresses so the process layout never crosses the
/// FFI boundary.
#[cfg(all(feature = "python", any(test, feature = "test-util")))]
#[pymethods]
impl FemtoLogger {
    /// Return one opaque identity token per attached handler.
    ///
    /// Tokens follow attachment order, so a list captured before a
    /// reconfiguration can be compared with one captured afterwards without
    /// exposing native addresses. The token class is deliberately not
    /// registered with the Python module.
    fn handler_identities_for_test(&self, py: Python<'_>) -> PyResult<Vec<Py<AttachmentIdentity>>> {
        let handlers = self.handlers.read();
        handlers
            .iter()
            .map(|attachment| {
                Py::new(
                    py,
                    AttachmentIdentity::for_handler(Arc::downgrade(&attachment.handler)),
                )
            })
            .collect()
    }

    /// Return one opaque identity token per attached filter.
    ///
    /// Tokens follow attachment order and compare by allocation identity, as
    /// [`Self::handler_identities_for_test`] does for handlers.
    fn filter_identities_for_test(&self, py: Python<'_>) -> PyResult<Vec<Py<AttachmentIdentity>>> {
        let filters = self.filters.read();
        filters
            .iter()
            .map(|filter| Py::new(py, AttachmentIdentity::for_filter(Arc::downgrade(filter))))
            .collect()
    }
}

/// Detachment seam for callers that already hold the interpreter.
///
/// The barrier in [`super::flush`] names no `pyo3` type, so releasing the GIL
/// belongs here, at the Python boundary: this module owns the
/// `Python<'_>` token every Python-facing flush already carries, and the
/// log-compat manager sweep in [`crate::manager::registry`] shares the same
/// entry point rather than detaching for itself.
impl FemtoLogger {
    /// Release Python while the shard and handlers consume the flush deadline.
    ///
    /// The final verdict is still recorded after the interpreter is
    /// reacquired, so a Python return that lands past the deadline reports
    /// `false` even when the Rust-side barrier completed in time.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// fn py_flush(&self, py: Python<'_>) -> bool {
    ///     self.flush_handlers_detached(py)
    /// }
    /// ```
    pub(crate) fn flush_handlers_detached(&self, py: Python<'_>) -> bool {
        #[cfg(not(loom))]
        let outcome = py.detach(|| self.flush_handlers_blocking());
        #[cfg(loom)]
        let outcome = {
            let _ = py;
            self.flush_handlers_blocking()
        };
        outcome.finish(
            &self.flush_diagnostics,
            FlushFailureStage::CallerReturnDeadlineExceeded,
        )
    }
}

/// How many times `remove_handler` scans before reporting no removable match.
///
/// A scan tries every match it found, so repeated re-entry cannot exhaust a
/// budget measured in matching attachments. The bound only limits rescans
/// after a re-entrant caller removed everything the previous scan found, and
/// stops a handler whose `as_any` always does that from spinning forever.
pub(crate) const MAX_REMOVAL_SCANS: usize = 4;

impl FemtoLogger {
    /// Select every attached handler wrapping *handler*, with no guard held.
    ///
    /// `as_any` is a trait method any handler may override, so calling it under
    /// this logger's lock would deadlock a handler that re-enters the logger.
    /// The clones keep the scan valid without the guard.
    fn select_py_handlers(
        &self,
        py: Python<'_>,
        handler: &Py<PyAny>,
    ) -> Vec<Arc<dyn FemtoHandlerTrait>> {
        let candidates: Vec<Arc<dyn FemtoHandlerTrait>> = {
            let handlers = self.handlers.read();
            handlers
                .iter()
                .map(|attachment| Arc::clone(&attachment.handler))
                .collect()
        };
        candidates
            .into_iter()
            .filter(|candidate| {
                candidate
                    .as_any()
                    .downcast_ref::<PyHandler>()
                    .is_some_and(|py_h| py_h.obj.bind(py).is(handler.bind(py)))
            })
            .collect()
    }

    /// Remove the attachment holding *target*, reporting whether it was there.
    ///
    /// Take the removed attachment out of the guarded block: dropping it inside
    /// would run a Python finalizer that may re-enter this logger and take the
    /// same write lock. The drop happens after the guard is gone but still
    /// inside `Python::attach`, so decref is immediate.
    fn remove_selected_handler(&self, target: &Arc<dyn FemtoHandlerTrait>) -> bool {
        let removed = {
            let mut handlers = self.handlers.write();
            handlers
                .iter()
                .position(|attachment| Arc::ptr_eq(&attachment.handler, target))
                .map(|pos| handlers.remove(pos))
        };
        let is_removed = removed.is_some();
        drop(removed);
        is_removed
    }
}
