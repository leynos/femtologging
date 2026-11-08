//! Assertions for bounded producer tracing event fields.

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
