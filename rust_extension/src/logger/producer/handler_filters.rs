//! Handler-filter evaluation on the logging producer thread.

use std::sync::Arc;

use crate::filters::{FemtoFilter, FilterContext};
use crate::handler::FemtoHandlerTrait;
use crate::log_record::FemtoLogRecord;

use super::super::{HandlerAttachment, HandlerRecordSnapshot};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FilterChainOutcome {
    Accepted,
    Rejected,
    CallbackFailed,
}

impl FilterChainOutcome {
    #[cfg(feature = "tracing-compat")]
    fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::CallbackFailed => "callback_failed",
        }
    }
}

#[cfg(feature = "tracing-compat")]
pub(super) fn handler_kind(handlers: &[Arc<dyn FemtoHandlerTrait>]) -> &'static str {
    let has_python = handlers.iter().any(|handler| handler.is_python_backed());
    let has_native = handlers.iter().any(|handler| !handler.is_python_backed());
    match (has_python, has_native) {
        (true, true) => "python_and_native",
        (true, false) => "python",
        (false, true) => "native",
        (false, false) => "none",
    }
}

/// Apply one filter chain to a record on the producer thread.
pub(super) fn apply_filter_chain(
    record: &mut FemtoLogRecord,
    filters: &[Arc<dyn FemtoFilter>],
) -> FilterChainOutcome {
    let mut context = FilterContext::default();
    for filter in filters {
        let decision = filter.decision(record, &mut context);
        if decision.callback_failed {
            return FilterChainOutcome::CallbackFailed;
        }
        if !decision.accepted {
            return FilterChainOutcome::Rejected;
        }
        record.metadata_mut().key_values.extend(decision.enrichment);
    }
    FilterChainOutcome::Accepted
}

/// Retain handlers whose producer-thread filter chains accept the record.
pub(super) fn filter_handlers(
    record: &mut FemtoLogRecord,
    handlers: Vec<HandlerAttachment>,
) -> Vec<Arc<dyn FemtoHandlerTrait>> {
    let has_handler_filters = handlers
        .iter()
        .any(|attachment| !attachment.filters.is_empty());
    handlers
        .into_iter()
        .filter_map(|attachment| filter_handler_attachment(record, attachment, has_handler_filters))
        .collect()
}

fn filter_handler_attachment(
    record: &mut FemtoLogRecord,
    attachment: HandlerAttachment,
    snapshot_required: bool,
) -> Option<Arc<dyn FemtoHandlerTrait>> {
    #[cfg(feature = "tracing-compat")]
    let filter_started = (!attachment.filters.is_empty()).then(std::time::Instant::now);
    #[cfg(feature = "tracing-compat")]
    let handler_kind = if attachment.handler.is_python_backed() {
        "python"
    } else {
        "native"
    };
    let outcome = apply_filter_chain(record, &attachment.filters);
    #[cfg(feature = "tracing-compat")]
    if let Some(filter_started) = filter_started {
        tracing::trace!(
            target: "femtologging::internal",
            operation = "producer_handler_filter_chain",
            handler_kind,
            filter_outcome = outcome.as_str(),
            elapsed_us = filter_started.elapsed().as_micros(),
        );
    }

    (outcome == FilterChainOutcome::Accepted).then(|| {
        if snapshot_required {
            HandlerRecordSnapshot::snapshot_handler(attachment.handler, record.clone())
        } else {
            attachment.handler
        }
    })
}
