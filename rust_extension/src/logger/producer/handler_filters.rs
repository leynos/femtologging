//! Handler-filter evaluation on the logging producer thread.

use std::sync::Arc;

use crate::filters::{FemtoFilter, FilterContext};
use crate::handler::FemtoHandlerTrait;
use crate::log_record::FemtoLogRecord;

use super::super::{HandlerAttachment, HandlerRecordSnapshot};

/// Result of evaluating one handler's producer-thread filter chain.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FilterChainOutcome {
    /// Every filter accepted the record.
    Accepted,
    /// A filter rejected the record.
    Rejected,
    /// A callback filter failed while evaluating the record.
    CallbackFailed,
}

impl FilterChainOutcome {
    /// Return the bounded label used in internal trace events.
    #[cfg(feature = "tracing-compat")]
    const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::CallbackFailed => "callback_failed",
        }
    }
}

/// Classify a set of handlers for bounded producer trace fields.
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
    apply_filter_segment(record, filters, &mut FilterContext::default())
}

/// Apply one contiguous segment of a filter chain, reusing `context`.
///
/// The deferred-capture split evaluates two segments of the same chain around
/// the capture hook, so both must share one [`FilterContext`]: a Python record
/// view built for one filter stays available to the next.
pub(super) fn apply_filter_segment(
    record: &mut FemtoLogRecord,
    filters: &[Arc<dyn FemtoFilter>],
    context: &mut FilterContext,
) -> FilterChainOutcome {
    for filter in filters {
        let decision = filter.decision(record, context);
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

/// Evaluate one attachment and capture its record state if accepted.
fn filter_handler_attachment(
    record: &mut FemtoLogRecord,
    attachment: HandlerAttachment,
    snapshot_required: bool,
) -> Option<Arc<dyn FemtoHandlerTrait>> {
    #[cfg(feature = "tracing-compat")]
    let filter_started = (!attachment.filters.is_empty()
        && tracing::enabled!(target: "femtologging::internal", tracing::Level::TRACE))
    .then(std::time::Instant::now);
    #[cfg(feature = "tracing-compat")]
    let handler_kind = if attachment.handler.is_python_backed() {
        "python"
    } else {
        "native"
    };
    let outcome = apply_filter_chain(record, &attachment.filters);
    #[cfg(feature = "tracing-compat")]
    if let Some(started_at) = filter_started {
        tracing::trace!(
            target: "femtologging::internal",
            operation = "producer_handler_filter_chain",
            handler_kind,
            filter_outcome = outcome.as_str(),
            elapsed_us = started_at.elapsed().as_micros(),
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

// `FilterChainOutcome::as_str` is `const`, and it labels a bounded trace field.
// It is only compiled with the tracing bridge, and it is private to this
// module, so an external `trybuild` fixture cannot name it.
//
// The call is evaluated in a const item, which is what enforces the qualifier:
// a binding of the function pointer would accept a non-`const` `fn` just as
// readily, so it would pin the signature and not the const-ness.
#[cfg(all(test, feature = "tracing-compat"))]
const _: &str = FilterChainOutcome::Accepted.as_str();
#[cfg(all(test, feature = "tracing-compat"))]
const _: &str = FilterChainOutcome::Rejected.as_str();
#[cfg(all(test, feature = "tracing-compat"))]
const _: &str = FilterChainOutcome::CallbackFailed.as_str();
