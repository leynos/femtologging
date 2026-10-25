//! Non-blocking admission of prepared records to their dispatch shard.

use log::warn;

#[cfg(not(loom))]
use super::super::DispatcherError;
use super::super::{FemtoLogger, QueuedRecord};

impl FemtoLogger {
    /// Admit one prepared record and report a refused submission as dropped.
    pub(super) fn admit_local_record(
        &self,
        queued: Box<QueuedRecord>,
        #[cfg(feature = "tracing-compat")] handler_kind: &'static str,
    ) {
        #[cfg(feature = "tracing-compat")]
        let dispatch_started = std::time::Instant::now();
        #[cfg(not(loom))]
        let accepted = match self.dispatch_state() {
            Ok((shard, budget)) => shard.admit(queued, budget).is_ok(),
            Err(error) => {
                self.record_dispatch_start_failure(&error);
                return;
            }
        };
        #[cfg(loom)]
        let accepted = self.shard.admit(queued, &self.budget).is_ok();

        if accepted {
            #[cfg(feature = "tracing-compat")]
            tracing::trace!(
                target: "femtologging::internal",
                operation = "producer_queue_dispatch",
                handler_kind,
                dispatch_outcome = "queued",
                elapsed_us = dispatch_started.elapsed().as_micros(),
            );
            return;
        }
        #[cfg(feature = "tracing-compat")]
        tracing::trace!(
            target: "femtologging::internal",
            operation = "producer_queue_dispatch",
            handler_kind,
            dispatch_outcome = "rejected",
            elapsed_us = dispatch_started.elapsed().as_micros(),
        );
        self.dropped_records
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.drop_warner.record_drop();
        self.drop_warner.warn_if_due(|count| {
            warn!("FemtoLogger: dropped {count} records; dispatch capacity exhausted");
        });
    }

    /// Count a record refused because a post-fork pool could not start.
    #[cfg(not(loom))]
    fn record_dispatch_start_failure(&self, error: &DispatcherError) {
        self.dropped_records
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.drop_warner.record_drop();
        self.drop_warner.warn_if_due(|count| {
            warn!("FemtoLogger: dropped {count} records; could not start the process-local dispatch pool: {error}");
        });
    }
}
