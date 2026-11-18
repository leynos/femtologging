//! `remove_handler` tests where the removal scan itself re-enters the logger.
//!
//! Selection inspects handlers outside the collection lock, because a handler's
//! `as_any` is arbitrary code that may call back into the logger. That leaves a
//! window between selecting an attachment and removing it, and a re-entrant
//! caller can take the selected attachment first. Removal must then look again
//! rather than report that no match exists while another one does.

use std::any::Any;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use pyo3::types::PyModule;

use super::*;
use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::log_record::FemtoLogRecord;

/// Handler whose `as_any` removes one Python attachment during the scan.
struct ReentrantRemovalProbe {
    /// Logger to re-enter; weak, so the handler never keeps it alive.
    logger: Weak<FemtoLogger>,
    /// The Python handler object the outer call is looking for.
    target: Py<PyAny>,
    /// Whether the re-entry has already happened.
    tripped: AtomicBool,
}

impl FemtoHandlerTrait for ReentrantRemovalProbe {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        // Trip once: the inner call scans this same collection and would
        // re-enter here without the flag.
        if !self.tripped.swap(true, Ordering::SeqCst)
            && let Some(logger) = self.logger.upgrade()
        {
            Python::attach(|py| {
                assert!(
                    logger.py_remove_handler(self.target.clone_ref(py)),
                    "the re-entrant call must remove the first attachment",
                );
            });
        }
        self
    }
}

/// Build the Python object two attachments will wrap.
fn handler_object(py: Python<'_>) -> Py<PyAny> {
    let module = PyModule::from_code(
        py,
        c"class Handler:\n    def handle(self, record):\n        pass\n",
        c"reentrant_removal.py",
        c"reentrant_removal",
    )
    .expect("the test handler module compiles");
    module
        .getattr("Handler")
        .expect("the class exists")
        .call0()
        .expect("the handler constructs")
        .unbind()
}

/// A re-entrant scan must not hide a matching attachment from the caller.
#[test]
fn removal_retries_after_a_reentrant_scan_removes_the_selected_attachment() {
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
                tripped: AtomicBool::new(false),
            }))
            .expect("the probe attaches");
        for _ in 0..2 {
            logger
                .add_handler(Arc::new(PyHandler::new(py, object.clone_ref(py))))
                .expect("the Python handler attaches");
        }

        assert!(
            logger.py_remove_handler(object),
            "a matching attachment remained, so removal must report success"
        );
        assert_eq!(
            logger.handlers.read().len(),
            1,
            "the re-entrant scan and the outer call each remove one attachment",
        );
    });
}
