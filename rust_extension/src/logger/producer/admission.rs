//! Non-blocking admission of prepared records to their dispatch shard.

use log::warn;

#[cfg(not(loom))]
use super::super::DispatcherError;
use super::super::dispatcher::AdmissionRefusal;
use super::super::{FemtoLogger, QueuedRecord};

/// Report whether a shard admission was accepted.
///
/// A refusal hands the record back, so dropping it here is what releases the
/// record's handlers; the caller then counts and warns about the dropped
/// record. Taking the record out of the refusal also keeps the payload in use
/// in the library build, where the dispatch path never inspects it.
///
/// `UnexpectedJob` cannot be produced by a real send: `Shard::admit` sends only
/// records, and the variant exists to keep its match total. A test therefore
/// passes it in directly, which is why this is `pub(super)` rather than private
/// to the module — the arm is otherwise unreachable from any test at all.
pub(super) fn is_admitted(admission: Result<(), AdmissionRefusal>) -> bool {
    match admission {
        Ok(()) => true,
        Err(AdmissionRefusal::Record(refused)) => {
            drop(refused);
            false
        }
        Err(AdmissionRefusal::UnexpectedJob) => false,
    }
}

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
            Ok((shard, budget)) => is_admitted(shard.admit(queued, budget)),
            Err(error) => {
                self.record_dispatch_start_failure(&error);
                return;
            }
        };
        #[cfg(loom)]
        let accepted = is_admitted(self.shard.admit(queued, &self.budget));

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

#[cfg(all(test, not(loom)))]
#[path = "admission_tests.rs"]
mod admission_tests;
