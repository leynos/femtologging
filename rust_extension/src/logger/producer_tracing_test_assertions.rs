//! Assertions for bounded producer tracing event fields.
//!
//! The dispatch trace events carry a fixed field vocabulary, and these helpers
//! are the check that it stays fixed: they select the events for one dispatch
//! outcome and assert both the count the scenario should produce and the exact
//! key set.
//!
//! [`assert_no_local_handler_events`] is called by
//! `producer_tracing_tests::context_dispatch_tracing_reports_bounded_outcomes`
//! for the two records dispatched with no attached handler, and
//! [`assert_no_handlers_accepted_events`] for the one record every handler
//! filter rejected. Keeping the assertions here rather than inline lets both
//! outcomes share the "exact keys, no payload" check without that test growing
//! past its own scenario.

use std::collections::BTreeSet;

use super::helpers::CapturedEvent;

/// Assert events emitted when no attached handler accepts a record.
pub(super) fn assert_no_local_handler_events(events: &[CapturedEvent]) {
    let no_local_handler_events = events
        .iter()
        .filter(|event| {
            event.fields.get("dispatch_outcome").map(String::as_str) == Some("no_local_handlers")
        })
        .collect::<Vec<_>>();
    assert_eq!(no_local_handler_events.len(), 2);
    for event in no_local_handler_events {
        assert_eq!(event.fields["handler_kind"], "none");
        assert_eq!(event.fields["dispatch_outcome"], "no_local_handlers");
        assert_eq!(
            event
                .fields
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["dispatch_outcome", "handler_kind", "operation",]),
            "no-handler trace fields must stay bounded: {:?}",
            event.fields,
        );
    }
}

/// Assert events emitted when handler filters reject a record.
pub(super) fn assert_no_handlers_accepted_events(events: &[CapturedEvent]) {
    let no_handlers_accepted_events = events
        .iter()
        .filter(|event| {
            event.fields.get("dispatch_outcome").map(String::as_str) == Some("no_handlers_accepted")
        })
        .collect::<Vec<_>>();
    assert_eq!(no_handlers_accepted_events.len(), 1);
    for event in no_handlers_accepted_events {
        assert_eq!(event.fields["operation"], "producer_handler_dispatch");
        assert_eq!(event.fields["handler_kind"], "none");
        assert_eq!(
            event
                .fields
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["dispatch_outcome", "handler_kind", "operation",]),
            "all-rejected trace fields must stay bounded: {:?}",
            event.fields,
        );
    }
}
