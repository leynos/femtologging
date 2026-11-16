// Expected producer and worker tracing values, kept in a data file beside the
// helper module.
//
// `producer_tracing_test_helpers` includes this file with `include!` rather
// than reading and parsing it at run time: the tracing test caller passes
// `&EXPECTED_EVENTS` as a slice, so the compile-time array type is part of the
// helper's contract. Keeping the literals here leaves the assertion helpers
// readable without changing the type, the values, or their order.

/// Expected identifying fields and occurrence count for a tracing event.
pub(super) const EXPECTED_EVENTS: [ExpectedEvent; 8] = [
    ExpectedEvent {
        operation: "producer_context_capture",
        handler_kind: "python",
        outcome_field: "capture_outcome",
        outcome: "captured",
        count: 2,
    },
    ExpectedEvent {
        operation: "producer_queue_dispatch",
        handler_kind: "python",
        outcome_field: "dispatch_outcome",
        outcome: "queued",
        count: 2,
    },
    ExpectedEvent {
        operation: "producer_context_capture",
        handler_kind: "python",
        outcome_field: "capture_outcome",
        outcome: "failed",
        count: 1,
    },
    ExpectedEvent {
        operation: "producer_context_capture",
        handler_kind: "native",
        outcome_field: "capture_outcome",
        outcome: "not_required",
        count: 3,
    },
    ExpectedEvent {
        operation: "producer_queue_dispatch",
        handler_kind: "native",
        outcome_field: "dispatch_outcome",
        outcome: "rejected",
        count: 1,
    },
    ExpectedEvent {
        operation: "producer_queue_dispatch",
        handler_kind: "native",
        outcome_field: "dispatch_outcome",
        outcome: "queued",
        count: 2,
    },
    ExpectedEvent {
        operation: "worker_handler_dispatch",
        handler_kind: "python",
        outcome_field: "dispatch_outcome",
        outcome: "success",
        count: 1,
    },
    ExpectedEvent {
        operation: "worker_handler_dispatch",
        handler_kind: "python",
        outcome_field: "dispatch_outcome",
        outcome: "failed",
        count: 1,
    },
];

/// Expected per-chain counts for the accepted, rejected, and failed outcomes.
pub(super) const EXPECTED_FILTER_OUTCOMES: [(&str, usize); 3] =
    [("accepted", 3), ("rejected", 3), ("callback_failed", 1)];