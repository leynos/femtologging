//! Manager tests that prove guards are released before runtime resources drop.
//!
//! A handler or filter held by the runtime snapshot may be the last owner of a
//! Python object. Destroying it under `MANAGER` would let a finalizer re-enter
//! `get_logger` or `reset_manager` and deadlock on the same lock. These probes
//! record whether `MANAGER` was free at the instant their `Drop` ran.
//!
//! The same rule covers the values a configuration commit builds but cannot
//! install: a refused commit and a batch that fails partway must both hand
//! their values to an unlocked path rather than destroy them under the guard.

use std::any::Any;
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use pyo3::{Python, exceptions::PyRuntimeError};
use serial_test::serial;

use super::registry::{MANAGER, build_batch};
use super::{
    MAX_LOGGERS, PreparedConfiguration, PreparedLogger, commit_configuration,
    replace_runtime_state, reset_manager, stage_loggers,
};
use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::log_record::FemtoLogRecord;
use crate::logger::HandlerAttachment;

/// Verdict recorded when the runtime probe is destroyed.
#[derive(Default)]
struct DropObservation {
    /// Whether `MANAGER` could be taken non-blockingly.
    lock_available: AtomicBool,
    /// Number of probes destroyed.
    drops: AtomicUsize,
}

impl DropObservation {
    /// Return the recorded lock verdict.
    fn lock_was_available(&self) -> bool {
        self.lock_available.load(Ordering::SeqCst)
    }

    /// Return how many probes were destroyed.
    fn drop_count(&self) -> usize {
        self.drops.load(Ordering::SeqCst)
    }
}

/// Handler probe that checks the manager lock when it is dropped.
struct ManagerLockProbe {
    /// Verdict recorded by `Drop`.
    observation: Arc<DropObservation>,
}

impl FemtoHandlerTrait for ManagerLockProbe {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Drop for ManagerLockProbe {
    fn drop(&mut self) {
        // `try_write` fails for both a writer and an outer reader, unlike
        // `try_read`, which would miss a guard the probe's own thread holds.
        self.observation
            .lock_available
            .store(MANAGER.try_write().is_some(), Ordering::SeqCst);
        self.observation.drops.fetch_add(1, Ordering::SeqCst);
    }
}

/// Publish one probe as the sole owner held by the runtime registry.
fn publish_probe(observation: &Arc<DropObservation>) {
    let mut handlers: BTreeMap<String, Arc<dyn FemtoHandlerTrait>> = BTreeMap::new();
    handlers.insert(
        "probe".to_string(),
        Arc::new(ManagerLockProbe {
            observation: Arc::clone(observation),
        }),
    );
    let displaced =
        replace_runtime_state(handlers, BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    // The first replacement displaces an empty snapshot.
    drop(displaced);
}

#[test]
#[serial]
fn reset_manager_drops_runtime_state_after_releasing_the_manager_lock() {
    Python::attach(|_py| {
        reset_manager();
        let observation = Arc::new(DropObservation::default());
        publish_probe(&observation);

        reset_manager();

        assert_eq!(observation.drop_count(), 1, "the probe must be dropped");
        assert!(
            observation.lock_was_available(),
            "reset_manager must release MANAGER before dropping runtime state"
        );
    });
}

#[test]
#[serial]
fn replace_runtime_state_returns_displaced_state_dropped_without_the_lock() {
    Python::attach(|_py| {
        reset_manager();
        let observation = Arc::new(DropObservation::default());
        publish_probe(&observation);

        let displaced = replace_runtime_state(
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        );
        // The swap must hand the previous snapshot back intact; destroying it
        // is the caller's step, not part of the locked replacement.
        assert_eq!(
            observation.drop_count(),
            0,
            "replace_runtime_state must return the displaced state without dropping it"
        );
        drop(displaced);

        assert_eq!(observation.drop_count(), 1, "the probe must be dropped");
        assert!(
            observation.lock_was_available(),
            "replace_runtime_state must not drop displaced state under MANAGER"
        );
    });
}

/// A batch that fails partway hands its values back for the caller to drop.
///
/// A capacity refusal builds nothing, so it cannot show what happens to a value
/// that already exists when construction fails. This case injects that failure
/// after the first value and pins both halves of the rule: the builder does not
/// drop the partial batch, and the caller destroys it only after releasing the
/// manager guard.
///
/// The production caller, `register_missing_ancestors`, cannot fail on a second
/// name without a test seam in `FemtoLogger` construction, so this drives the
/// shared `build_batch` directly. The commit path adds only its own guard
/// disposal around the same hand-back, which a seam would not otherwise prove.
#[test]
#[serial]
fn a_partial_batch_is_returned_for_the_caller_to_drop() {
    Python::attach(|_py| {
        reset_manager();
        let observation = Arc::new(DropObservation::default());

        let manager = MANAGER.write();
        let result = build_batch(
            ["first", "second"].into_iter().map(str::to_string),
            |name| {
                if name == "first" {
                    Ok(ManagerLockProbe {
                        observation: Arc::clone(&observation),
                    })
                } else {
                    Err(PyRuntimeError::new_err("injected construction failure"))
                }
            },
        );

        let Err(refusal) = result else {
            panic!("the injected construction failure must be reported");
        };
        assert_eq!(
            refusal.built.len(),
            1,
            "the batch must carry the value it built before failing"
        );
        assert_eq!(
            observation.drop_count(),
            0,
            "the partial batch must outlive the caller's guard"
        );

        drop(manager);
        drop(refusal);

        assert_eq!(
            observation.drop_count(),
            1,
            "the caller drops the partial batch exactly once"
        );
        assert!(
            observation.lock_was_available(),
            "the partial batch must be destroyed after the manager guard is released"
        );
    });
}

/// A refused commit releases the manager lock before dropping staged handlers.
#[test]
#[serial]
fn a_refused_commit_drops_staged_handlers_after_unlocking() {
    Python::attach(|py| {
        reset_manager();
        let staged = stage_loggers(py, std::iter::once("probe"))
            .expect("one staged logger fits an empty registry");
        let probe_logger = staged
            .get("probe")
            .expect("the staged map holds the requested name")
            .clone_ref(py);

        let observation = Arc::new(DropObservation::default());
        let probe_plan = PreparedLogger {
            name: "refused-0".to_string(),
            logger: probe_logger.clone_ref(py),
            handlers: vec![HandlerAttachment::new(Arc::new(ManagerLockProbe {
                observation: Arc::clone(&observation),
            }))],
            filters: Vec::new(),
            handler_ids: Vec::new(),
            filter_ids: Vec::new(),
            level: None,
            propagate: None,
        };
        // One more distinct name than the registry can hold, so the commit is
        // refused after staging but before any plan is applied.
        let remaining = (1..=MAX_LOGGERS).map(|index| PreparedLogger {
            name: format!("refused-{index}"),
            logger: probe_logger.clone_ref(py),
            handlers: Vec::new(),
            filters: Vec::new(),
            handler_ids: Vec::new(),
            filter_ids: Vec::new(),
            level: None,
            propagate: None,
        });
        let loggers: Vec<PreparedLogger> = std::iter::once(probe_plan).chain(remaining).collect();

        let result = commit_configuration(
            py,
            PreparedConfiguration {
                loggers,
                disable_existing: false,
                keep_names: HashSet::new(),
                handler_registry: BTreeMap::new(),
                handler_filter_ids: BTreeMap::new(),
                filter_registry: BTreeMap::new(),
            },
        );

        assert!(result.is_err(), "a commit over the cap must be refused");
        assert!(
            observation.lock_was_available(),
            "staged handlers must be dropped after the manager lock is released",
        );
        assert!(
            MANAGER.read().loggers.is_empty(),
            "a refused commit must not register anything",
        );
    });
}

/// A commit that fails partway hands its built ancestors to an unlocked drop.
///
/// The capacity refusal builds nothing, so it cannot show what happens to an
/// ancestor that already exists when construction fails. The injected seam
/// makes the second name fail, which is the branch `register_missing_ancestors`
/// exists to serve: the partial batch must leave the guarded scope intact and
/// be destroyed only after the manager guard is released.
#[test]
#[serial]
fn a_commit_that_fails_partway_drops_built_ancestors_after_unlocking() {
    use super::registry::set_injected_construction_failure;

    Python::attach(|py| {
        reset_manager();
        // The staged name supplies the chain; `deep` then `deep.middle` are the
        // missing ancestors, and the second one is made to fail.
        let staged = stage_loggers(py, std::iter::once("deep.middle.leaf"))
            .expect("one staged logger fits an empty registry");
        let staged_logger = staged
            .get("deep.middle.leaf")
            .expect("the staged map holds the requested name")
            .clone_ref(py);
        let previous = set_injected_construction_failure(Some("deep.middle".to_string()));

        let result = commit_configuration(
            py,
            PreparedConfiguration {
                loggers: vec![PreparedLogger {
                    name: "deep.middle.leaf".to_string(),
                    logger: staged_logger,
                    handlers: Vec::new(),
                    filters: Vec::new(),
                    handler_ids: Vec::new(),
                    filter_ids: Vec::new(),
                    level: None,
                    propagate: None,
                }],
                disable_existing: false,
                keep_names: HashSet::new(),
                handler_registry: BTreeMap::new(),
                handler_filter_ids: BTreeMap::new(),
                filter_registry: BTreeMap::new(),
            },
        );
        set_injected_construction_failure(previous);

        assert!(
            result.is_err(),
            "the injected failure must refuse the commit"
        );
        // The probe recorded the lock state at the instant the batch dropped,
        // so this fails if the batch is destroyed under the manager guard.
        assert!(
            super::registry::ANCESTOR_DROP_SAW_FREE_LOCK.load(Ordering::SeqCst),
            "the partial batch must be destroyed after MANAGER is released",
        );
        assert_eq!(
            super::registry::ANCESTOR_DROPS.load(Ordering::SeqCst),
            1,
            "the ancestor built before the failure must be destroyed exactly once",
        );
        assert!(
            MANAGER.read().loggers.is_empty(),
            "a commit that failed partway must register nothing",
        );
    });
}
