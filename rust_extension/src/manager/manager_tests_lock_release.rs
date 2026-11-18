//! Manager tests that prove guards are released before runtime resources drop.
//!
//! A handler or filter held by the runtime snapshot may be the last owner of a
//! Python object. Destroying it under `MANAGER` would let a finalizer re-enter
//! `get_logger` or `reset_manager` and deadlock on the same lock. These probes
//! record whether `MANAGER` was free at the instant their `Drop` ran.

use std::any::Any;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use pyo3::Python;
use serial_test::serial;

use super::registry::MANAGER;
use super::{replace_runtime_state, reset_manager};
use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::log_record::FemtoLogRecord;

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
