//! Shared test helpers for logger unit tests.
//!
//! Provides reusable handler implementations used across the logger
//! test modules.

use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;
use parking_lot::Mutex;
use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Default)]
pub(super) struct CollectingHandler {
    pub(super) records: Arc<Mutex<Vec<FemtoLogRecord>>>,
}

#[derive(Clone, Copy)]
pub(super) struct HandlePtr(pub(super) *const Mutex<Option<std::thread::JoinHandle<()>>>);

impl HandlePtr {
    pub(super) unsafe fn as_ref<'a>(self) -> &'a Mutex<Option<std::thread::JoinHandle<()>>> {
        // SAFETY: Callers guarantee the pointee outlives this reference.
        unsafe { &*self.0 }
    }
}

// SAFETY: The pointee is a `parking_lot::Mutex`, which is `Send + Sync` and
// all access goes through `as_ref()` to obtain shared references that rely on
// the mutex for interior mutability. Callers must also guarantee the pointee
// outlives any use of `HandlePtr`.
unsafe impl Send for HandlePtr {}
unsafe impl Sync for HandlePtr {}

impl CollectingHandler {
    pub(super) fn new() -> Self {
        Self {
            records: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn collected(&self) -> Vec<FemtoLogRecord> {
        self.records.lock().clone()
    }
}

impl FemtoHandlerTrait for CollectingHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        self.records.lock().push(record);
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone, Default)]
pub(super) struct CountingHandler {
    pub(super) count: Arc<AtomicUsize>,
    pub(super) first_tx: Option<crossbeam_channel::Sender<()>>,
}

impl CountingHandler {
    pub(super) fn with_first_signal(first_tx: crossbeam_channel::Sender<()>) -> Self {
        Self {
            count: Arc::new(AtomicUsize::new(0)),
            first_tx: Some(first_tx),
        }
    }
}

impl FemtoHandlerTrait for CountingHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        if self.count.fetch_add(1, Ordering::SeqCst) == 0 {
            if let Some(first_tx) = &self.first_tx {
                let _ = first_tx.send(());
            }
        }
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
