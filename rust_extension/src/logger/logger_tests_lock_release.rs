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
use rstest::rstest;
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

/// Record whether `lock` was free when a probe that owns it was destroyed.
///
/// A failed upgrade means the owning logger is already gone; in this module that
/// would make the test meaningless, so `DropObservation::record` treats it as
/// held.
fn record_guard_was_free<T>(lock: &Weak<RwLock<T>>, observation: &DropObservation) {
    let verdict = lock.upgrade().map(|lock| lock.try_write().is_some());
    observation.record(verdict);
}

/// Build a logger named for the scenario under test.
fn test_logger(name: &str) -> FemtoLogger {
    FemtoLogger::new(name.into()).expect("test logger should construct")
}

/// Assert a probe was destroyed once and that its owning guard was free then.
fn assert_dropped_after_release(observation: &DropObservation, operation: &str) {
    assert_eq!(
        observation.drop_count(),
        1,
        "{operation} must drop the probe exactly once"
    );
    assert!(
        observation.lock_was_available(),
        "{operation} must release the collection lock before dropping values"
    );
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
        record_guard_was_free(&self.handlers, &self.observation);
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
        record_guard_was_free(&self.filters, &self.observation);
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
        record_guard_was_free(&self.handlers, &self.observation);
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

/// A logger collection that can own user resources.
#[derive(Clone, Copy)]
enum CollectionKind {
    /// The logger's handler collection.
    Handlers,
    /// The logger's filter collection.
    Filters,
}

impl CollectionKind {
    /// Attach a probe, leaving this collection as the probe's only owner so a
    /// later mutation destroys it.
    fn attach_probe(self, logger: &FemtoLogger) -> Arc<DropObservation> {
        match self {
            Self::Handlers => attach_handler_probe(logger),
            Self::Filters => attach_filter_probe(logger),
        }
    }

    /// Empty this collection.
    fn clear(self, logger: &FemtoLogger) {
        match self {
            Self::Handlers => logger.clear_handlers(),
            Self::Filters => logger.clear_filters(),
        }
    }

    /// Name the clear operation in assertion messages.
    fn clearing_operation(self) -> &'static str {
        match self {
            Self::Handlers => "clear_handlers",
            Self::Filters => "clear_filters",
        }
    }

    /// Replace this collection with an empty one, dropping what it displaced.
    #[cfg(feature = "python")]
    fn replace_with_nothing(self, logger: &FemtoLogger) {
        // Dropping the displaced value here, outside any guard, mirrors how the
        // production call sites dispose of it.
        match self {
            Self::Handlers => drop(logger.replace_handlers(Vec::new())),
            Self::Filters => drop(logger.replace_filters(Vec::new())),
        }
    }

    /// Name the replace operation in assertion messages.
    #[cfg(feature = "python")]
    fn replacing_operation(self) -> &'static str {
        match self {
            Self::Handlers => "replace_handlers",
            Self::Filters => "replace_filters",
        }
    }
}

/// Attach a probe to *logger*'s handler collection and leave that collection as
/// the probe's only owner.
fn attach_handler_probe(logger: &FemtoLogger) -> Arc<DropObservation> {
    let (probe, observation) = handler_probe(logger);
    logger
        .add_handler(probe.clone() as Arc<dyn FemtoHandlerTrait>)
        .expect("probe handler should attach");
    drop(probe);
    observation
}

/// Attach a probe to *logger*'s filter collection and leave that collection as
/// the probe's only owner.
fn attach_filter_probe(logger: &FemtoLogger) -> Arc<DropObservation> {
    let (probe, observation) = filter_probe(logger);
    logger.add_filter(probe.clone() as Arc<dyn FemtoFilter>);
    drop(probe);
    observation
}

#[rstest]
#[case::handlers(CollectionKind::Handlers)]
#[case::filters(CollectionKind::Filters)]
fn clear_drops_values_after_releasing_the_lock(#[case] kind: CollectionKind) {
    let logger = test_logger("lock.clear");
    let observation = kind.attach_probe(&logger);

    kind.clear(&logger);

    assert_dropped_after_release(&observation, kind.clearing_operation());
}

#[cfg(feature = "python")]
#[test]
fn remove_handler_drops_the_attachment_filter_chain_after_releasing_the_lock() {
    let logger = test_logger("lock.remove.handler");
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

    assert_dropped_after_release(&observation, "remove_handler");
}

/// Removal detaches the collection's reference; the caller's outlives the call.
#[test]
fn remove_filter_releases_the_collections_reference() {
    let logger = test_logger("lock.remove.filter");
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

    assert_dropped_after_release(&observation, "remove_filter");
}

// `replace_handlers` and `replace_filters` live in the python-gated
// `runtime_mutation` module. Test builds currently always enable the feature
// through the crate's self dev-dependency, but the gate keeps this file honest
// if that changes.
#[cfg(feature = "python")]
#[rstest]
#[case::handlers(CollectionKind::Handlers)]
#[case::filters(CollectionKind::Filters)]
fn replace_drops_displaced_values_after_releasing_the_lock(#[case] kind: CollectionKind) {
    let logger = test_logger("lock.replace");
    let observation = kind.attach_probe(&logger);

    kind.replace_with_nothing(&logger);

    assert_dropped_after_release(&observation, kind.replacing_operation());
}
