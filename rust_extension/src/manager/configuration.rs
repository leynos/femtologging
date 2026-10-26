//! Prepared logger configuration and its single manager commit.
//!
//! The configuration builder resolves every fallible reference before calling
//! the commit. This module publishes the resolved logger states and retains
//! displaced resources until the manager lock has been released.

use std::{
    collections::{BTreeMap, HashMap, hash_map::Entry},
    sync::Arc,
};

use pyo3::prelude::*;

use crate::{
    filters::FemtoFilter,
    level::FemtoLevel,
    logger::{FemtoLogger, HandlerAttachment},
};

use super::{LoggerAttachmentState, MANAGER, RuntimeStateSnapshot, SharedFilters, SharedHandlers};

/// One logger's fully resolved configuration, ready for commit.
///
/// Every object reference is resolved during preparation so the commit takes
/// no name lookups and performs no fallible work.
pub(crate) struct PreparedLogger {
    /// Name the plan applies to.
    pub(crate) name: String,
    /// Registry handle for an existing logger, or a new unregistered object.
    pub(crate) logger: Py<FemtoLogger>,
    /// Resolved handlers to attach, replacing the current set.
    pub(crate) handlers: Vec<HandlerAttachment>,
    /// Resolved filters to attach, replacing the current set.
    pub(crate) filters: Vec<Arc<dyn FemtoFilter>>,
    /// Handler identifiers recorded as runtime attachment metadata.
    pub(crate) handler_ids: Vec<String>,
    /// Filter identifiers recorded as runtime attachment metadata.
    pub(crate) filter_ids: Vec<String>,
    /// Level to apply, or `None` to leave the current level in place.
    pub(crate) level: Option<FemtoLevel>,
    /// Propagation flag to apply, or `None` to leave the current value.
    pub(crate) propagate: Option<bool>,
}

/// The complete post-configuration state awaiting one atomic commit.
pub(crate) struct PreparedConfiguration {
    /// Logger plans, applied in order.
    pub(crate) loggers: Vec<PreparedLogger>,
    /// Whether loggers omitted from this configuration should be disabled.
    pub(crate) disable_existing: bool,
    /// Logger names and ancestors protected from disabling.
    pub(crate) keep_names: std::collections::HashSet<String>,
    /// Handler registry to publish.
    pub(crate) handler_registry: SharedHandlers,
    /// Filter IDs associated with each handler in the registry.
    pub(crate) handler_filter_ids: BTreeMap<String, Vec<String>>,
    /// Filter registry to publish.
    pub(crate) filter_registry: SharedFilters,
}

/// Resources displaced by commit and retained until its lock and GIL are released.
#[derive(Default)]
pub(crate) struct DisplacedAttachments {
    /// Handler attachments removed from loggers by the commit.
    pub(crate) handlers: Vec<HandlerAttachment>,
    /// Filters removed from loggers by the commit.
    pub(crate) filters: Vec<Arc<dyn FemtoFilter>>,
    /// Staged logger handles not inserted because their names were registered
    /// by another caller before commit.
    pub(crate) unused_loggers: Vec<Py<FemtoLogger>>,
    /// The previous runtime registry snapshot, retained until after unlock.
    pub(crate) runtime_state: Option<RuntimeStateSnapshot>,
}

impl DisplacedAttachments {
    /// Drop displaced resources without the GIL after all Python borrows end.
    pub(crate) fn drop_without_gil(self, py: Python<'_>) {
        py.detach(move || drop(self));
    }
}

/// Identity and registry handle used while adopting a staged logger.
struct StagedLogger<'a> {
    /// Name used as the manager registry key.
    name: &'a str,
    /// Logger object prepared before the manager lock was acquired.
    logger: Py<FemtoLogger>,
}

/// Fully resolved values that replace one logger's current state.
struct LoggerStateUpdate {
    /// Handler attachments that replace the current handlers.
    handlers: Vec<HandlerAttachment>,
    /// Filters that replace the current filters.
    filters: Vec<Arc<dyn FemtoFilter>>,
    /// Optional level update for the logger.
    level: Option<FemtoLevel>,
    /// Optional propagation update for the logger.
    propagate: Option<bool>,
}

/// Apply a fully prepared configuration as one atomic transition.
///
/// Preparation has validated every logger name, resource reference and scalar
/// setting. This function takes the manager write lock once, performs no name
/// lookups and returns no `Result`.
///
/// The root and every logger name in the keep set are exempt from disabling.
/// Omitted targets are disabled before their attachments are cleared. The live
/// registry is consulted under the lock so registrations between preparation
/// and commit are included.
#[cfg(feature = "python")]
pub(crate) fn commit_configuration(
    py: Python<'_>,
    prepared: PreparedConfiguration,
) -> DisplacedAttachments {
    let mut displaced = DisplacedAttachments::default();
    let mut manager = MANAGER.write();
    let PreparedConfiguration {
        loggers,
        disable_existing,
        keep_names,
        handler_registry,
        handler_filter_ids,
        filter_registry,
    } = prepared;
    let mut logger_states = BTreeMap::new();

    for plan in loggers {
        let (name, state) = apply_prepared_logger(py, &mut manager.loggers, plan, &mut displaced);
        logger_states.insert(name, state);
    }

    if disable_existing {
        disable_omitted_loggers(py, &manager.loggers, &keep_names, &mut displaced);
    }

    displaced.runtime_state = Some(std::mem::replace(
        &mut manager.runtime,
        RuntimeStateSnapshot {
            handler_registry,
            handler_filter_ids,
            filter_registry,
            logger_states,
        },
    ));
    drop(manager);
    displaced
}

/// Apply one resolved plan to its adopted or newly registered logger.
fn apply_prepared_logger(
    py: Python<'_>,
    loggers: &mut HashMap<String, Py<FemtoLogger>>,
    plan: PreparedLogger,
    displaced: &mut DisplacedAttachments,
) -> (String, LoggerAttachmentState) {
    let PreparedLogger {
        name,
        logger: staged,
        handlers,
        filters,
        handler_ids,
        filter_ids,
        level,
        propagate,
    } = plan;
    let logger = adopt_logger(
        py,
        loggers,
        StagedLogger {
            name: &name,
            logger: staged,
        },
        displaced,
    );
    apply_logger_plan(
        py,
        &logger,
        LoggerStateUpdate {
            handlers,
            filters,
            level,
            propagate,
        },
        displaced,
    );

    (name, LoggerAttachmentState::new(handler_ids, filter_ids))
}

/// Insert a staged logger or retain a concurrently registered logger instead.
fn adopt_logger(
    py: Python<'_>,
    loggers: &mut HashMap<String, Py<FemtoLogger>>,
    staged: StagedLogger<'_>,
    displaced: &mut DisplacedAttachments,
) -> Py<FemtoLogger> {
    match loggers.entry(staged.name.to_owned()) {
        Entry::Occupied(entry) => {
            // Defer the unused staged worker's teardown until after unlock.
            displaced.unused_loggers.push(staged.logger);
            entry.get().clone_ref(py)
        }
        Entry::Vacant(entry) => entry.insert(staged.logger).clone_ref(py),
    }
}

/// Replace the logger's prepared state and retain displaced attachments.
fn apply_logger_plan(
    py: Python<'_>,
    logger: &Py<FemtoLogger>,
    update: LoggerStateUpdate,
    displaced: &mut DisplacedAttachments,
) {
    let LoggerStateUpdate {
        handlers,
        filters,
        level,
        propagate,
    } = update;
    let logger = logger.borrow(py);
    let mut old_handlers = logger.replace_handlers(handlers);
    displaced.handlers.append(&mut old_handlers);
    let mut old_filters = logger.replace_filters(filters);
    displaced.filters.append(&mut old_filters);
    if let Some(level) = level {
        logger.set_level(level);
    }
    if let Some(propagate) = propagate {
        logger.set_propagate(propagate);
    }
    // Re-enable only loggers named explicitly in this full configuration.
    logger.set_disabled(false);
}

/// Disable omitted loggers outside the root and keep set, clearing attachments.
fn disable_omitted_loggers(
    py: Python<'_>,
    loggers: &HashMap<String, Py<FemtoLogger>>,
    keep_names: &std::collections::HashSet<String>,
    displaced: &mut DisplacedAttachments,
) {
    for (name, logger) in loggers {
        if name == "root" || keep_names.contains(name) {
            continue;
        }
        let logger = logger.borrow(py);
        logger.set_disabled(true);
        let mut old_handlers = logger.replace_handlers(Vec::new());
        displaced.handlers.append(&mut old_handlers);
        let mut old_filters = logger.replace_filters(Vec::new());
        displaced.filters.append(&mut old_filters);
    }
}

#[cfg(test)]
mod tests {
    //! Verify displaced resources are dropped after the Python GIL is released.

    use std::{
        any::Any,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        thread,
        time::Duration,
    };

    use pyo3::Python;

    use crate::{
        handler::{FemtoHandlerTrait, HandlerError},
        log_record::FemtoLogRecord,
    };

    use super::DisplacedAttachments;

    /// Handler that checks whether another thread can attach during teardown.
    struct GILProbeHandler {
        /// Records whether another thread attached to Python during `Drop`.
        gil_was_released: Arc<AtomicBool>,
    }

    impl Drop for GILProbeHandler {
        fn drop(&mut self) {
            let (sender, receiver) = mpsc::channel();
            let worker = thread::Builder::new().spawn(move || {
                Python::attach(|_| ());
                let _ = sender.send(());
            });
            let Ok(worker) = worker else {
                return;
            };

            let acquired_gil = receiver.recv_timeout(Duration::from_secs(5)).is_ok();
            let worker_finished = if acquired_gil {
                worker.join().is_ok()
            } else {
                drop(worker);
                false
            };
            self.gil_was_released
                .store(acquired_gil && worker_finished, Ordering::SeqCst);
        }
    }

    impl FemtoHandlerTrait for GILProbeHandler {
        fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
            Ok(())
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    #[test]
    fn displaced_attachments_drop_without_the_gil() {
        Python::attach(|py| {
            let gil_was_released = Arc::new(AtomicBool::new(false));
            let handler = Arc::new(GILProbeHandler {
                gil_was_released: gil_was_released.clone(),
            });
            let displaced = DisplacedAttachments {
                handlers: vec![crate::logger::HandlerAttachment::new(handler)],
                ..DisplacedAttachments::default()
            };

            displaced.drop_without_gil(py);

            assert!(
                gil_was_released.load(Ordering::SeqCst),
                "handler teardown should run while another thread can attach to Python",
            );
        });
    }
}
