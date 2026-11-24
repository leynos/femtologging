//! Python `remove_handler` tests: re-entrant scans and guard release.
//!
//! Selection inspects handlers outside the collection lock, because a handler's
//! `as_any` is arbitrary code that may call back into the logger. That leaves a
//! window between selecting an attachment and removing it, and a re-entrant
//! caller can take selected attachments first. Removal must try every match the
//! scan found rather than report that no match exists while another one does.
//!
//! The Python route also destroys the removed attachment itself, so a probe in
//! its filter chain is a genuine last-owner drop inside `py_remove_handler`.

use std::any::Any;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Weak};

use proptest::prelude::*;
use pyo3::types::PyModule;
use rstest::rstest;

use super::logger_tests_lock_release::{
    AttachmentFilterLockProbe, DropObservation, assert_dropped_after_release, test_logger,
};
use super::*;
use crate::filters::FemtoFilter;
use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::log_record::FemtoLogRecord;

/// Handler whose outermost `as_any` removes Python attachments on each scan.
struct ReentrantRemovalProbe {
    /// Logger to re-enter; weak, so the handler never keeps it alive.
    logger: Weak<FemtoLogger>,
    /// The Python handler object the outer call is looking for.
    target: Py<PyAny>,
    /// Number of attachments each outermost scan tries to remove.
    removals_per_scan: usize,
    /// Successful re-entrant removals, shared with the test.
    reentrant_removals: Arc<AtomicUsize>,
    /// Number of `as_any` calls currently on this thread's stack.
    depth: AtomicUsize,
}

impl FemtoHandlerTrait for ReentrantRemovalProbe {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        // Only the outermost scan re-enters: the call it makes scans this same
        // collection, so tripping there too would recurse without bound.
        let depth = self.depth.fetch_add(1, Ordering::SeqCst);
        if depth == 0 {
            self.remove_budgeted_matches();
        }
        self.depth.fetch_sub(1, Ordering::SeqCst);
        self
    }
}

impl ReentrantRemovalProbe {
    /// Re-enter removal for each attempt this probe was given.
    ///
    /// A call removes one match when one remains. The nested scan cannot
    /// re-enter because the depth is non-zero, so a call that finds nothing
    /// simply reports `false`; the test reconciles the tally rather than
    /// assuming how many matches a scan leaves.
    fn remove_budgeted_matches(&self) {
        let Some(logger) = self.logger.upgrade() else {
            return;
        };
        Python::attach(|py| {
            for _ in 0..self.removals_per_scan {
                let removed = logger.py_remove_handler(self.target.clone_ref(py));
                if removed {
                    self.reentrant_removals.fetch_add(1, Ordering::SeqCst);
                }
            }
        });
    }
}

/// Build the Python object the attachments wrap.
fn handler_object(py: Python<'_>) -> Py<PyAny> {
    let module = PyModule::from_code(
        py,
        c"class Handler:\n    def handle(self, record):\n        pass\n",
        c"python_removal.py",
        c"python_removal",
    )
    .expect("the test handler module compiles");
    module
        .getattr("Handler")
        .expect("the class exists")
        .call0()
        .expect("the handler constructs")
        .unbind()
}

/// A scan that loses attachments must not hide a remaining match.
///
/// Each scan here loses one attachment to the re-entrant call. Selecting a
/// single target per attempt would therefore exhaust its budget after four
/// matches; trying every target from one scan keeps the outcome independent of
/// how many matches exist.
#[rstest]
#[case::two_attachments(2)]
#[case::six_attachments(6)]
fn removal_keeps_progress_when_a_scan_loses_attachments(#[case] attachments: usize) {
    Python::attach(|py| {
        let logger = Arc::new(
            FemtoLogger::new("reentrant.remove".into()).expect("test logger should construct"),
        );
        let object = handler_object(py);

        // Attached first, so its `as_any` runs before the Python attachments
        // are inspected and its re-entry wins the race deterministically.
        logger
            .add_handler(Arc::new(ReentrantRemovalProbe {
                logger: Arc::downgrade(&logger),
                target: object.clone_ref(py),
                removals_per_scan: 1,
                reentrant_removals: Arc::new(AtomicUsize::new(0)),
                depth: AtomicUsize::new(0),
            }))
            .expect("the probe attaches");
        for _ in 0..attachments {
            logger
                .add_handler(Arc::new(PyHandler::new(py, object.clone_ref(py))))
                .expect("the Python handler attaches");
        }

        assert!(
            logger.py_remove_handler(object),
            "matching attachments remained, so removal must report success"
        );
        assert_eq!(
            logger.handlers.read().len(),
            attachments - 1,
            "the re-entrant scan and the outer call each remove one attachment, \
             whatever the match count",
        );
    });
}

/// Run one generated removal scenario and check the invariant.
///
/// Separated from the `proptest!` block so the arrangement does not sit three
/// closures deep; `proptest!` would otherwise nest its body inside the test
/// module, the `Python::attach` call and its own wrapper.
fn check_removal_progress(attachments: usize, removals: usize) -> Result<(), TestCaseError> {
    let removals = removals.min(attachments);
    Python::attach(|py| {
        let logger = Arc::new(
            FemtoLogger::new("reentrant.remove.property".into())
                .expect("test logger should construct"),
        );
        let object = handler_object(py);
        let reentrant_removals = Arc::new(AtomicUsize::new(0));
        logger
            .add_handler(Arc::new(ReentrantRemovalProbe {
                logger: Arc::downgrade(&logger),
                target: object.clone_ref(py),
                removals_per_scan: removals,
                reentrant_removals: Arc::clone(&reentrant_removals),
                depth: AtomicUsize::new(0),
            }))
            .expect("the probe attaches");
        for _ in 0..attachments {
            logger
                .add_handler(Arc::new(PyHandler::new(py, object.clone_ref(py))))
                .expect("the Python handler attaches");
        }

        // The re-entry happens inside the call, so read its tally after.
        let removed = logger.py_remove_handler(object);
        let taken = reentrant_removals.load(Ordering::SeqCst);
        let expected_success = taken < attachments;
        prop_assert_eq!(
            removed,
            expected_success,
            "with {} attachments and {} taken by re-entry, success must be {}",
            attachments,
            taken,
            expected_success,
        );
        let expected_survivors = if expected_success {
            attachments - taken - 1
        } else {
            0
        };
        prop_assert_eq!(
            logger.handlers.read().len() - 1,
            expected_survivors,
            "the surviving matches must be those neither the re-entry nor the \
             outer call took",
        );
        Ok(())
    })
}

proptest! {
    /// Removal reports success exactly when a match survives the re-entry.
    ///
    /// The invariant: whatever the number of matching attachments and however
    /// many removals the re-entrant scan attempts, `py_remove_handler` reports
    /// success exactly when a match is still there for it, and the surviving
    /// count is the matches neither the re-entry nor the outer call took. The
    /// re-entry's actual tally is read back rather than assumed, so the case
    /// where it consumes every match is covered too.
    #[test]
    fn removal_reports_success_for_any_match_count(
        attachments in 2usize..=16,
        removals in 0usize..=16,
    ) {
        check_removal_progress(attachments, removals)?;
    }
}

/// `py_remove_handler` drops its attachment's filter chain after the guard.
///
/// The attachment's filter chain is the probe's only owner, so removing the
/// attachment is a last-owner drop; its destructor must find the handler lock
/// free even though the removal was requested through the Python method.
#[test]
fn py_removal_drops_the_attachment_filter_chain_after_releasing_the_lock() {
    Python::attach(|py| {
        let logger = test_logger("lock.remove.py_handler");
        let observation = DropObservation::new();
        let probe = AttachmentFilterLockProbe::new(&logger, Arc::clone(&observation));
        let object = handler_object(py);
        logger
            .add_handler_with_filters(
                Arc::new(PyHandler::new(py, object.clone_ref(py))) as Arc<dyn FemtoHandlerTrait>,
                vec![probe as Arc<dyn FemtoFilter>],
            )
            .expect("attachment with a probe filter should attach");

        assert!(
            logger.py_remove_handler(object),
            "the Python attachment must be found",
        );
        assert_dropped_after_release(&observation, "py_remove_handler");
    });
}
