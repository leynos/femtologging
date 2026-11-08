//! Shared test helpers for logger unit tests.
//!
//! Provides the handler implementations the logger test modules share.
//! `CollectingHandler` is re-exported from the crate-wide `test_utils` module;
//! the signalling variant lives here.

use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::log_record::FemtoLogRecord;
use crossbeam_channel::{Receiver, Sender, bounded};
use rstest::fixture;
use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

pub(super) use crate::test_utils::collecting_handler::CollectingHandler;

const RECORD_WAIT_TIMEOUT: Duration = Duration::from_secs(1);

#[fixture]
pub(super) fn collecting_handler() -> Arc<CollectingHandler> {
    Arc::new(CollectingHandler::new())
}

/// Return the messages collected by `handler`, in arrival order.
pub(super) fn collected_messages(handler: &CollectingHandler) -> Vec<String> {
    handler
        .collected()
        .iter()
        .map(|record| record.message().to_owned())
        .collect()
}

/// A collecting handler that also signals on a channel as it handles.
#[derive(Clone)]
pub(super) struct SignallingCollectingHandler {
    /// Collector that retains each record delivered to this handler.
    inner: Arc<CollectingHandler>,
    /// Channel used to notify the test after a record is handled.
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

    fn flush_until(&self, _deadline: std::time::Instant) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Wait for a handler to signal that it has handled a record.
pub(super) fn wait_for_record_signal(
    record_rx: &Receiver<()>,
) -> Result<(), crossbeam_channel::RecvTimeoutError> {
    record_rx.recv_timeout(RECORD_WAIT_TIMEOUT)
}
