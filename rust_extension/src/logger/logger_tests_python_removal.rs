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
use crate::logger::python_api::MAX_REMOVAL_SCANS;

/// Handler whose outermost `as_any` removes one Python attachment per scan.
struct ReentrantRemovalProbe {
    /// Logger to re-enter; weak, so the handler never keeps it alive.
    logger: Weak<FemtoLogger>,
    /// The Python handler object the outer call is looking for.
    target: Py<PyAny>,
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
        if depth == 0
            && let Some(logger) = self.logger.upgrade()
        {
            Python::attach(|py| {
                assert!(
                    logger.py_remove_handler(self.target.clone_ref(py)),
                    "the re-entrant call must remove the first attachment",
                );
            });
        }
        self.depth.fetch_sub(1, Ordering::SeqCst);
        self
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

proptest! {
    /// Removal reports success exactly when a match survives the re-entry.
    ///
    /// The invariant: for any number of matching attachments,
    /// `py_remove_handler` returns `true` whenever at least one match remains
    /// after the re-entrant scan, and the surviving count is what the re-entry
    /// did not take. Re-entry is bounded by `MAX_REMOVAL_SCANS`, so the
    /// generator stays inside the range where the bound cannot be the reason
    /// for a `false`.
    #[test]
    fn removal_reports_success_for_any_match_count(attachments in 2usize..=16) {
        Python::attach(|py| {
            let logger = Arc::new(
                FemtoLogger::new("reentrant.remove.property".into())
                    .expect("test logger should construct"),
            );
            let object = handler_object(py);
            logger
                .add_handler(Arc::new(ReentrantRemovalProbe {
                    logger: Arc::downgrade(&logger),
                    target: object.clone_ref(py),
                    depth: AtomicUsize::new(0),
                }))
                .expect("the probe attaches");
            for _ in 0..attachments {
                logger
                    .add_handler(Arc::new(PyHandler::new(py, object.clone_ref(py))))
                    .expect("the Python handler attaches");
            }

            prop_assert!(
                logger.py_remove_handler(object),
                "removal must report success while {} matches remain, within the \
                 {} scan bound",
                attachments - 1,
                MAX_REMOVAL_SCANS,
            );
            prop_assert_eq!(
                logger.handlers.read().len(),
                attachments - 1,
                "the surviving count must be the matches the re-entry did not take",
            );
            Ok(())
        })?;
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
