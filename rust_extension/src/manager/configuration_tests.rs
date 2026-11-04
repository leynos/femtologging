//! Verify displaced configuration resources are dropped after the GIL release.

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
