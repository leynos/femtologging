//! Shared test helpers for logger unit tests.
//!
//! Provides reusable handler implementations used across the logger
//! test modules.  `CollectingHandler` is re-exported from the
//! crate-wide `test_utils` module; logger-specific helpers
//! (`HandlePtr`, `CountingHandler`) are defined here.

use super::QueuedRecord;
use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;
use crossbeam_channel::{Receiver, SendError, Sender, bounded};
use parking_lot::Mutex;
use rstest::fixture;
use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

pub(super) use crate::test_utils::collecting_handler::CollectingHandler;

const RECORD_WAIT_TIMEOUT: Duration = Duration::from_secs(1);

/// Non-owning pointer to a worker join handle protected by a mutex.
#[derive(Clone, Copy)]
pub(super) struct HandlePtr(
    /// Mutex containing the worker's optional join handle.
    pub(super) *const Mutex<Option<std::thread::JoinHandle<()>>>,
);

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

#[fixture]
pub(super) fn collecting_handler() -> Arc<CollectingHandler> {
    Arc::new(CollectingHandler::new())
}

/// Enqueue one `QueuedRecord` per entry in `messages` on `tx`, all
/// addressed to `handler`.
///
/// Fallible so callers can `.expect()` at the panic boundary; the send only
/// fails if the receiving end has been dropped. Box the returned error because
/// it owns the unsent record and would otherwise inflate every test's result.
pub(super) fn enqueue_records(
    tx: &Sender<QueuedRecord>,
    handler: &Arc<dyn FemtoHandlerTrait>,
    messages: &[&str],
) -> Result<(), Box<SendError<QueuedRecord>>> {
    for message in messages {
        tx.send(QueuedRecord {
            record: FemtoLogRecord::new("core", FemtoLevel::Info, message),
            handlers: vec![handler.clone()],
            #[cfg(feature = "python")]
            context: None,
        })
        .map_err(Box::new)?;
    }
    Ok(())
}

/// Return the messages collected by `handler`, in arrival order.
pub(super) fn collected_messages(handler: &CollectingHandler) -> Vec<String> {
    handler
        .collected()
        .iter()
        .map(|record| record.message().to_owned())
        .collect()
}

/// Handler that stores records and signals after each successful capture.
#[derive(Clone)]
pub(super) struct SignallingCollectingHandler {
    /// Shared collector that retains the handled records.
    inner: Arc<CollectingHandler>,
    /// Notifies the test that a record was captured.
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
        let _ = self.record_tx.try_send(());
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub(super) fn wait_for_record_signal(
    record_rx: &Receiver<()>,
) -> Result<(), crossbeam_channel::RecvTimeoutError> {
    record_rx.recv_timeout(RECORD_WAIT_TIMEOUT)
}

/// Handler that counts records and can signal when the first arrives.
#[derive(Clone, Default)]
pub(super) struct CountingHandler {
    /// Shared number of records handled by this value and its clones.
    pub(super) count: Arc<AtomicUsize>,
    /// Optional one-shot notification sent after the first record.
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
        if self.count.fetch_add(1, Ordering::SeqCst) == 0
            && let Some(first_tx) = &self.first_tx
        {
            let _ = first_tx.send(());
        }
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
