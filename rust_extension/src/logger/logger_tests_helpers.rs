//! Shared test helpers for logger unit tests.
//!
//! Provides reusable handler implementations used across the logger
//! test modules.  `CollectingHandler` is re-exported from the
//! crate-wide `test_utils` module; logger-specific helpers
//! (`HandlePtr`, `CountingHandler`) are defined here.

use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::log_record::FemtoLogRecord;
use crossbeam_channel::{Receiver, Sender, bounded};
use parking_lot::Mutex;
use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

pub(super) use crate::test_utils::collecting_handler::CollectingHandler;

const RECORD_WAIT_TIMEOUT: Duration = Duration::from_secs(1);

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

pub(super) fn collecting_handler() -> Arc<CollectingHandler> {
    Arc::new(CollectingHandler::new())
}

#[derive(Clone)]
pub(super) struct SignallingCollectingHandler {
    inner: Arc<CollectingHandler>,
    record_tx: Sender<()>,
}

impl SignallingCollectingHandler {
    pub(super) fn with_signal() -> (Arc<CollectingHandler>, Self, Receiver<()>) {
        let inner = collecting_handler();
        let (record_tx, record_rx) = bounded(1);
        (Arc::clone(&inner), Self { inner, record_tx }, record_rx)
    }
}

impl FemtoHandlerTrait for SignallingCollectingHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        self.inner.handle(record)?;
        let _ = self.record_tx.send(());
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub(super) fn wait_for_record_signal(record_rx: &Receiver<()>) {
    record_rx
        .recv_timeout(RECORD_WAIT_TIMEOUT)
        .expect("timed out waiting for queued record");
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
