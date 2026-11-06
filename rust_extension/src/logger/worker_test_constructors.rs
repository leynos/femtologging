//! Test-only constructors for [`FemtoLogger`].
//!
//! These seams exist because a logger's formatter, context snapshot provider
//! and shard are fixed at construction. Tests that must observe or control one
//! of them need a constructor rather than a public setter, and keeping them
//! beside the production constructors would push that module past its size
//! limit.

use crate::formatter::SharedFormatter;
#[cfg(feature = "python")]
use crate::logger::context_snapshot::ContextSnapshotProvider;
use crate::logger::dispatcher::Dispatcher;

use super::{FemtoLogger, ShardHandle};

impl FemtoLogger {
    /// Create a logger whose producer formatter a test supplies.
    ///
    /// A logger's formatter is fixed at construction, so a test that must
    /// observe whether the producer path reached formatting needs this seam
    /// rather than a public setter.
    #[cfg(feature = "python")]
    pub(crate) fn with_formatter_for_test(name: String, formatter: SharedFormatter) -> Self {
        let mut logger = Self::with_parent(name, None).expect("test dispatcher should start");
        logger.formatter = formatter;
        logger
    }

    /// Create a logger with a test-controlled context snapshot provider.
    #[cfg(feature = "python")]
    pub(in crate::logger) fn with_context_snapshot_provider(
        name: String,
        parent: Option<String>,
        context_snapshot_provider: std::sync::Arc<dyn ContextSnapshotProvider>,
    ) -> Self {
        let dispatcher = Dispatcher::shared().expect("test dispatcher should start");
        Self::assembled(
            name,
            parent,
            dispatcher.assign_shard(),
            context_snapshot_provider,
        )
    }

    /// Assemble a logger on a shard a test has chosen.
    ///
    /// Tests that assert what one logger's flush does to a shard it *shares*
    /// have to place the logger on that shard; the process-wide pool assigns
    /// round robin and gives no control over which logger lands where. Outside
    /// Loom only, like the pool it borrows from.
    pub(crate) fn assembled_for_test(name: String, shard: ShardHandle) -> Self {
        Self::from_shard(name, None, shard)
    }
}
