//! Global registry mapping logger names to instances.
//!
//! Access is guarded by a `parking_lot::RwLock` and must only occur while the
//! Python GIL is held. This ensures `Py<FemtoLogger>` objects remain valid.

use once_cell::sync::Lazy;
use parking_lot::RwLock;
use pyo3::prelude::*;
use std::collections::{HashMap, hash_map::Entry};
#[cfg(feature = "python")]
use std::{collections::BTreeMap, sync::Arc};

use crate::logger::FemtoLogger;
#[cfg(feature = "python")]
use crate::{filters::FemtoFilter, handler::FemtoHandlerTrait};

#[cfg(feature = "python")]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LoggerAttachmentState {
    handler_ids: Vec<String>,
    filter_ids: Vec<String>,
}

#[cfg(feature = "python")]
impl LoggerAttachmentState {
    pub(crate) fn new(handler_ids: Vec<String>, filter_ids: Vec<String>) -> Self {
        Self {
            handler_ids,
            filter_ids,
        }
    }

    pub(crate) fn handler_ids(&self) -> &[String] {
        &self.handler_ids
    }

    pub(crate) fn filter_ids(&self) -> &[String] {
        &self.filter_ids
    }
}

#[cfg(feature = "python")]
type SharedHandlers = BTreeMap<String, Arc<dyn FemtoHandlerTrait>>;
#[cfg(feature = "python")]
type SharedFilters = BTreeMap<String, Arc<dyn FemtoFilter>>;

#[cfg(feature = "python")]
#[derive(Clone, Default)]
pub(crate) struct RuntimeStateSnapshot {
    pub(crate) handler_registry: SharedHandlers,
    pub(crate) filter_registry: SharedFilters,
    pub(crate) logger_states: BTreeMap<String, LoggerAttachmentState>,
}

#[derive(Default)]
struct Manager {
    loggers: HashMap<String, Py<FemtoLogger>>,
    #[cfg(feature = "python")]
    runtime: RuntimeStateSnapshot,
}

static MANAGER: Lazy<RwLock<Manager>> = Lazy::new(|| RwLock::new(Manager::default()));

#[cfg(feature = "python")]
fn clear_runtime_state(mgr: &mut Manager) {
    mgr.runtime = RuntimeStateSnapshot::default();
}

#[cfg(not(feature = "python"))]
fn clear_runtime_state(_mgr: &mut Manager) {}

/// Return `true` when the provided name is not a valid logger identifier.
///
/// A name is considered invalid when it is empty, begins or ends with a dot,
/// or contains consecutive dots which would create empty segments.
fn is_invalid_logger_name(name: &str) -> bool {
    name.is_empty()
        || name.starts_with('.')
        || name.ends_with('.')
        || name.split('.').any(|s| s.is_empty())
}

fn ensure_root_logger(py: Python<'_>, mgr: &mut Manager) -> PyResult<()> {
    if !mgr.loggers.contains_key("root") {
        let root = Py::new(py, FemtoLogger::with_parent("root".into(), None))?;
        mgr.loggers.insert("root".to_string(), root);
    }
    Ok(())
}

fn calculate_parent_name(name: &str) -> Option<String> {
    name.rsplit_once('.')
        .map(|(p, _)| p.to_string())
        .or_else(|| (name != "root").then(|| "root".to_string()))
}

/// Retrieve an existing logger or create one with a dotted-name parent.
pub fn get_logger(py: Python<'_>, name: &str) -> PyResult<Py<FemtoLogger>> {
    if is_invalid_logger_name(name) {
        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "logger name cannot be empty, start or end with '.', or contain consecutive dots",
        ));
    }

    let mut mgr = MANAGER.write();
    ensure_root_logger(py, &mut mgr)?;

    match mgr.loggers.entry(name.to_string()) {
        Entry::Occupied(o) => Ok(o.get().clone_ref(py)),
        Entry::Vacant(v) => {
            let parent_name = calculate_parent_name(name);
            let logger = Py::new(py, FemtoLogger::with_parent(name.to_string(), parent_name))?;
            v.insert(logger.clone_ref(py));
            Ok(logger)
        }
    }
}

#[cfg(feature = "python")]
pub(crate) fn snapshot_runtime_state() -> RuntimeStateSnapshot {
    MANAGER.read().runtime.clone()
}

#[cfg(feature = "python")]
pub(crate) fn replace_runtime_state(
    handler_registry: SharedHandlers,
    filter_registry: SharedFilters,
    logger_states: BTreeMap<String, LoggerAttachmentState>,
) {
    let mut mgr = MANAGER.write();
    mgr.runtime = RuntimeStateSnapshot {
        handler_registry,
        filter_registry,
        logger_states,
    };
}

/// Disable existing loggers not mentioned in the provided keep list.
///
/// Iterates through all loggers and clears handlers and filters for any
/// whose name is absent from `keep_names`.
#[cfg(feature = "python")]
pub fn disable_existing_loggers(
    py: Python<'_>,
    keep_names: &std::collections::HashSet<String>,
) -> PyResult<()> {
    let mgr = MANAGER.read();
    for (name, logger) in &mgr.loggers {
        if name != "root" && !keep_names.contains(name) {
            let logger_ref = logger.borrow(py);
            logger_ref.clear_handlers();
            logger_ref.clear_filters();
        }
    }
    Ok(())
}

/// Flush handlers attached to every registered logger.
///
/// Intended for use by the Rust `log` crate bridge; failures are ignored.
#[cfg(feature = "log-compat")]
pub(crate) fn flush_all_handlers(py: Python<'_>) {
    let mgr = MANAGER.read();
    for logger in mgr.loggers.values() {
        let _ = logger.borrow(py).flush_handlers();
    }
}

#[pyfunction]
pub fn reset_manager() {
    let mut mgr = MANAGER.write();
    mgr.loggers.clear();
    clear_runtime_state(&mut mgr);
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "log-compat")]
    mod log_compat {
        use std::any::Any;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        use pyo3::Python;
        use serial_test::serial;

        use super::super::{MANAGER, flush_all_handlers, get_logger, reset_manager};
        use crate::handler::{FemtoHandlerTrait, HandlerError};
        use crate::log_record::FemtoLogRecord;

        #[derive(Clone)]
        struct FlushCountingHandler {
            flushes: Arc<AtomicUsize>,
        }

        impl FemtoHandlerTrait for FlushCountingHandler {
            fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
                Ok(())
            }

            fn flush(&self) -> bool {
                self.flushes.fetch_add(1, Ordering::SeqCst);
                true
            }

            fn as_any(&self) -> &dyn Any {
                self
            }
        }

        #[test]
        #[serial]
        fn flush_all_handlers_flushes_loggers_with_handlers() {
            Python::attach(|py| {
                reset_manager();

                let flushes = Arc::new(AtomicUsize::new(0));
                let handler = Arc::new(FlushCountingHandler {
                    flushes: flushes.clone(),
                }) as Arc<dyn FemtoHandlerTrait>;

                let logger_a = get_logger(py, "bridge.flush.a").expect("logger created");
                let logger_b = get_logger(py, "bridge.flush.b").expect("logger created");
                logger_a.borrow(py).add_handler(handler.clone());
                logger_b.borrow(py).add_handler(handler.clone());

                flush_all_handlers(py);

                assert_eq!(
                    flushes.load(Ordering::SeqCst),
                    2,
                    "flush should be invoked once per logger with handlers",
                );
            });
        }

        #[test]
        #[serial]
        fn flush_all_handlers_invokes_flush_once_per_registered_logger() {
            Python::attach(|py| {
                reset_manager();

                // Populate the manager with multiple loggers (including parents).
                let _ = get_logger(py, "bridge.flush.a").expect("logger created");
                let _ = get_logger(py, "bridge.flush.b").expect("logger created");

                let flushes = Arc::new(AtomicUsize::new(0));
                let handler = Arc::new(FlushCountingHandler {
                    flushes: flushes.clone(),
                }) as Arc<dyn FemtoHandlerTrait>;

                let loggers = {
                    let mgr = MANAGER.read();
                    mgr.loggers
                        .values()
                        .map(|logger| logger.clone_ref(py))
                        .collect::<Vec<_>>()
                };

                for logger in &loggers {
                    logger.borrow(py).add_handler(handler.clone());
                }

                flush_all_handlers(py);

                assert_eq!(
                    flushes.load(Ordering::SeqCst),
                    loggers.len(),
                    "flush should be invoked once per registered logger",
                );
            });
        }
    }
}
