//! Collection-mutation tests that prove the guard is released before a drop.
//!
//! A handler or filter may be the last owner of a Python object whose
//! finalizer re-enters this logger. The guard must therefore cover only the
//! collection mutation, never the destruction of the values it displaces.
//!
//! Each probe records whether the owning collection lock was free at the
//! instant its `Drop` ran. The probes never block or assert inside `Drop`;
//! they record a verdict the test thread reads afterwards.

use super::*;
use crate::filters::FemtoFilter;
use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::log_record::FemtoLogRecord;
use std::any::Any;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};

/// Verdict recorded when a probe is destroyed.
#[derive(Default)]
struct DropObservation {
    /// Whether the owning collection lock could be taken non-blockingly.
    lock_available: AtomicBool,
    /// Number of probe values destroyed.
    drops: AtomicUsize,
}

impl DropObservation {
    /// Create an observation that has recorded nothing yet.
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Record whether `lock` was uncontended at drop time.
    fn record(&self, lock: Option<bool>) {
        // A failed upgrade means the owning logger is already gone; in this
        // module that would make the test meaningless, so treat it as held.
        self.lock_available
            .store(lock.unwrap_or(false), Ordering::SeqCst);
        self.drops.fetch_add(1, Ordering::SeqCst);
    }

    /// Return the recorded lock verdict.
    fn lock_was_available(&self) -> bool {
        self.lock_available.load(Ordering::SeqCst)
    }

    /// Return how many probes were destroyed.
    fn drop_count(&self) -> usize {
        self.drops.load(Ordering::SeqCst)
    }
}

/// Handler probe that checks the logger's handler lock when it is dropped.
struct HandlerLockProbe {
    /// The collection that owns this probe.
    handlers: Weak<RwLock<Vec<HandlerAttachment>>>,
    /// Verdict recorded by `Drop`.
    observation: Arc<DropObservation>,
}

impl FemtoHandlerTrait for HandlerLockProbe {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Drop for HandlerLockProbe {
    fn drop(&mut self) {
        let probe = self
            .handlers
            .upgrade()
            .map(|lock| lock.try_write().is_some());
        self.observation.record(probe);
    }
}

/// Filter probe that checks the logger's filter lock when it is dropped.
struct FilterLockProbe {
    /// The collection that owns this probe.
    filters: Weak<RwLock<Vec<Arc<dyn FemtoFilter>>>>,
    /// Verdict recorded by `Drop`.
    observation: Arc<DropObservation>,
}

impl FemtoFilter for FilterLockProbe {
    fn decision(
        &self,
        _record: &mut FemtoLogRecord,
        _context: &mut crate::filters::FilterContext,
    ) -> crate::filters::FilterDecision {
        crate::filters::FilterDecision::accept(true)
    }
}

impl Drop for FilterLockProbe {
    fn drop(&mut self) {
        let probe = self
            .filters
            .upgrade()
            .map(|lock| lock.try_write().is_some());
        self.observation.record(probe);
    }
}

/// Filter probe that lives in an attachment's filter chain and observes the
/// logger's handler lock when it is dropped.
///
/// Removing an attachment drops its whole filter chain, so a probe placed here
/// is a genuine last-owner drop inside `remove_handler`.
struct AttachmentFilterLockProbe {
    /// The collection whose guard must already be released.
    handlers: Weak<RwLock<Vec<HandlerAttachment>>>,
    /// Verdict recorded by `Drop`.
    observation: Arc<DropObservation>,
}

impl FemtoFilter for AttachmentFilterLockProbe {
    fn decision(
        &self,
        _record: &mut FemtoLogRecord,
        _context: &mut crate::filters::FilterContext,
    ) -> crate::filters::FilterDecision {
        crate::filters::FilterDecision::accept(true)
    }
}

impl Drop for AttachmentFilterLockProbe {
    fn drop(&mut self) {
        let probe = self
            .handlers
            .upgrade()
            .map(|lock| lock.try_write().is_some());
        self.observation.record(probe);
    }
}

/// Handler probe that checks the handler lock but is only a stand-in owner.
///
/// Used when the finalizing resource is the attachment's filter chain rather
/// than its handler.
struct NoopHandler;

impl FemtoHandlerTrait for NoopHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Build a logger and a handler probe observing its handler collection.
fn handler_probe(logger: &FemtoLogger) -> (Arc<HandlerLockProbe>, Arc<DropObservation>) {
    let observation = DropObservation::new();
    let probe = Arc::new(HandlerLockProbe {
        handlers: Arc::downgrade(&logger.handlers),
        observation: Arc::clone(&observation),
    });
    (probe, observation)
}

/// Build a logger and a filter probe observing its filter collection.
fn filter_probe(logger: &FemtoLogger) -> (Arc<FilterLockProbe>, Arc<DropObservation>) {
    let observation = DropObservation::new();
    let probe = Arc::new(FilterLockProbe {
        filters: Arc::downgrade(&logger.filters),
        observation: Arc::clone(&observation),
    });
    (probe, observation)
}

#[test]
fn clear_handlers_drops_handlers_after_releasing_the_lock() {
    let logger =
        FemtoLogger::new("lock.clear.handlers".into()).expect("test logger should construct");
    let (probe, observation) = handler_probe(&logger);
    logger
        .add_handler(probe.clone() as Arc<dyn FemtoHandlerTrait>)
        .expect("probe handler should attach");
    // Leave the collection as the only owner so `clear_handlers` destroys it.
    drop(probe);

    logger.clear_handlers();

    assert_eq!(observation.drop_count(), 1, "the handler must be dropped");
    assert!(
        observation.lock_was_available(),
        "clear_handlers must release the handler lock before dropping values"
    );
}

#[test]
fn clear_filters_drops_filters_after_releasing_the_lock() {
    let logger =
        FemtoLogger::new("lock.clear.filters".into()).expect("test logger should construct");
    let (probe, observation) = filter_probe(&logger);
    logger.add_filter(probe.clone() as Arc<dyn FemtoFilter>);
    drop(probe);

    logger.clear_filters();

    assert_eq!(observation.drop_count(), 1, "the filter must be dropped");
    assert!(
        observation.lock_was_available(),
        "clear_filters must release the filter lock before dropping values"
    );
}

#[cfg(feature = "python")]
#[test]
fn remove_handler_drops_the_attachment_filter_chain_after_releasing_the_lock() {
    let logger =
        FemtoLogger::new("lock.remove.handler".into()).expect("test logger should construct");
    let observation = DropObservation::new();
    let probe = Arc::new(AttachmentFilterLockProbe {
        handlers: Arc::downgrade(&logger.handlers),
        observation: Arc::clone(&observation),
    });
    // The probe lives only inside the attachment's filter chain, so removing
    // the handler makes this drop the last owner.
    let handler: Arc<dyn FemtoHandlerTrait> = Arc::new(NoopHandler);
    logger
        .add_handler_with_filters(handler.clone(), vec![probe as Arc<dyn FemtoFilter>])
        .expect("attachment with a probe filter should attach");

    assert!(logger.remove_handler(&handler), "the handler must be found");

    assert_eq!(observation.drop_count(), 1, "the filter must be dropped");
    assert!(
        observation.lock_was_available(),
        "remove_handler must release the handler lock before dropping attachments"
    );
}

#[test]
fn remove_filter_drops_the_removed_reference_after_releasing_the_lock() {
    let logger =
        FemtoLogger::new("lock.remove.filter".into()).expect("test logger should construct");
    let (probe, observation) = filter_probe(&logger);
    // The caller must hold an `Arc` to name the filter, so removal cannot drop
    // the last owner. Keeping our own reference alive past the call proves the
    // eventual destruction still happens with the guard released, and that the
    // collection no longer owns the filter.
    let lookup: Arc<dyn FemtoFilter> = probe.clone();
    logger.add_filter(probe as Arc<dyn FemtoFilter>);

    assert!(logger.remove_filter(&lookup), "the filter must be found");
    assert_eq!(
        observation.drop_count(),
        0,
        "the caller's reference must keep the filter alive"
    );

    drop(lookup);

    assert_eq!(observation.drop_count(), 1, "the filter must be dropped");
    assert!(
        observation.lock_was_available(),
        "remove_filter must not leave the filter lock held once it returns"
    );
}

// `replace_handlers` lives in the python-gated `runtime_mutation` module. Test
// builds currently always enable the feature through the crate's self
// dev-dependency, but the gate keeps this file honest if that changes.
#[cfg(feature = "python")]
#[test]
fn replace_handlers_drops_displaced_handlers_after_releasing_the_lock() {
    let logger =
        FemtoLogger::new("lock.replace.handlers".into()).expect("test logger should construct");
    let (probe, observation) = handler_probe(&logger);
    logger
        .add_handler(probe.clone() as Arc<dyn FemtoHandlerTrait>)
        .expect("probe handler should attach");
    drop(probe);

    let displaced = logger.replace_handlers(Vec::new());
    drop(displaced);

    assert_eq!(observation.drop_count(), 1, "the handler must be dropped");
    assert!(
        observation.lock_was_available(),
        "replace_handlers must release the handler lock before dropping displaced values"
    );
}

#[cfg(feature = "python")]
#[test]
fn replace_filters_drops_displaced_filters_after_releasing_the_lock() {
    let logger =
        FemtoLogger::new("lock.replace.filters".into()).expect("test logger should construct");
    let (probe, observation) = filter_probe(&logger);
    logger.add_filter(probe.clone() as Arc<dyn FemtoFilter>);
    drop(probe);

    let displaced = logger.replace_filters(Vec::new());
    drop(displaced);

    assert_eq!(observation.drop_count(), 1, "the filter must be dropped");
    assert!(
        observation.lock_was_available(),
        "replace_filters must release the filter lock before dropping displaced values"
    );
}
