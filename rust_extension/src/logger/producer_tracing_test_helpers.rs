//! Shared field collection and assertions for producer tracing tests.

use std::collections::{BTreeMap, BTreeSet};

use tracing::field::{Field, Visit};

#[derive(Clone, Debug)]
pub(super) struct CapturedEvent {
    pub(super) fields: BTreeMap<String, String>,
}

#[derive(Default)]
struct EventFieldCollector {
    fields: BTreeMap<String, String>,
}

impl Visit for EventFieldCollector {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.fields
            .insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields
            .insert(field.name().to_owned(), value.to_owned());
    }
}

pub(super) fn collect_fields(event: &tracing::Event<'_>) -> BTreeMap<String, String> {
    let mut visitor = EventFieldCollector::default();
    event.record(&mut visitor);
    visitor.fields
}

pub(super) fn assert_event(
    event: &CapturedEvent,
    operation: &str,
    handler_kind: &str,
    outcome_field: &str,
    outcome: &str,
) {
    assert_eq!(
        event.fields.get("operation").map(String::as_str),
        Some(operation)
    );
    assert_eq!(
        event.fields.get("handler_kind").map(String::as_str),
        Some(handler_kind)
    );
    assert_eq!(
        event.fields.get(outcome_field).map(String::as_str),
        Some(outcome)
    );
    assert!(["python", "native", "python_and_native", "none"].contains(&handler_kind));

    let actual_fields = event
        .fields
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected_fields =
        BTreeSet::from(["elapsed_us", "handler_kind", "operation", outcome_field]);
    assert_eq!(
        actual_fields, expected_fields,
        "event fields: {:?}",
        event.fields
    );
    assert!(
        event.fields["elapsed_us"].parse::<u128>().is_ok(),
        "elapsed_us should be numeric: {:?}",
        event.fields
    );
}

pub(super) fn assert_events(
    events: &[CapturedEvent],
    expected: &[(&str, &str, &str, &str, usize)],
) {
    for (operation, handler_kind, outcome_field, outcome, expected_count) in expected {
        let matching = events
            .iter()
            .filter(|event| {
                event.fields.get("operation").map(String::as_str) == Some(*operation)
                    && event.fields.get("handler_kind").map(String::as_str) == Some(*handler_kind)
                    && event.fields.get(*outcome_field).map(String::as_str) == Some(*outcome)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            matching.len(),
            *expected_count,
            "expected {expected_count} {operation}/{handler_kind}/{outcome_field}={outcome} events; got {matching:?} in {events:?}"
        );
        for event in matching {
            assert_event(event, operation, handler_kind, outcome_field, outcome);
        }
    }
}
