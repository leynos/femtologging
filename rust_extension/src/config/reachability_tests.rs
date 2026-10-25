//! Unit tests for the reachability helper.
#![cfg(all(test, feature = "python"))]

use std::collections::BTreeMap;

use rstest::rstest;

use super::reachability::{
    ReachableIds, prune_filters, prune_handlers, unreachable_definition_error,
    unreachable_definition_ids,
};
use crate::{config::ConfigError, manager::LoggerAttachmentState};

fn state(handlers: &[&str], filters: &[&str]) -> LoggerAttachmentState {
    LoggerAttachmentState::new(
        handlers.iter().map(|id| (*id).to_string()).collect(),
        filters.iter().map(|id| (*id).to_string()).collect(),
    )
}

fn ids(handlers: &[&str], filters: &[&str]) -> ReachableIds {
    ReachableIds {
        handlers: handlers.iter().map(|id| (*id).to_string()).collect(),
        filters: filters.iter().map(|id| (*id).to_string()).collect(),
    }
}

#[rstest]
fn logger_states_union_every_logger() {
    let states = [state(&["a"], &["f1"]), state(&["b"], &[])];

    let reachable = ReachableIds::from_logger_states(states.iter());

    assert_eq!(
        reachable.handlers,
        ["a", "b"].map(str::to_string).into_iter().collect(),
        "every logger must contribute its handler identifiers, not just the first",
    );
    assert_eq!(
        reachable.filters,
        ["f1"].map(str::to_string).into_iter().collect(),
        "loggers without filters must not remove another logger's filter",
    );
}

#[rstest]
fn handler_filters_are_reachable_only_through_attached_handlers() {
    let states = [state(&["attached"], &[])];
    let handler_filter_ids = BTreeMap::from([
        ("attached".to_string(), vec!["used".to_string()]),
        ("detached".to_string(), vec!["unused".to_string()]),
    ]);

    let reachable =
        ReachableIds::from_logger_states_and_handler_filters(states.iter(), &handler_filter_ids);

    assert_eq!(
        reachable.handlers,
        ["attached"].map(str::to_string).into_iter().collect(),
        "only handlers in the logger graph are reachable",
    );
    assert_eq!(
        reachable.filters,
        ["used"].map(str::to_string).into_iter().collect(),
        "an attached handler retains its configured filters, while a detached handler does not",
    );
}

#[rstest]
fn logger_states_of_nothing_reach_nothing() {
    let states: [LoggerAttachmentState; 0] = [];

    let reachable = ReachableIds::from_logger_states(states.iter());

    assert_eq!(
        reachable,
        ReachableIds::default(),
        "an empty attachment graph must reach no identifiers at all",
    );
}

#[rstest]
fn prune_handlers_keeps_only_reachable_entries() {
    let mut registry = BTreeMap::from([("keep".to_string(), 1), ("drop".to_string(), 2)]);

    prune_handlers(&mut registry, &ids(&["keep"], &[]));

    assert_eq!(
        registry.keys().collect::<Vec<_>>(),
        vec!["keep"],
        "pruning must retain exactly the reachable handler entries",
    );
}

#[rstest]
fn prune_filters_keeps_only_reachable_entries() {
    let mut registry = BTreeMap::from([("keep".to_string(), 1), ("drop".to_string(), 2)]);

    prune_filters(&mut registry, &ids(&[], &["keep"]));

    assert_eq!(
        registry.keys().collect::<Vec<_>>(),
        vec!["keep"],
        "pruning must retain exactly the reachable filter entries",
    );
}

#[rstest]
fn prune_is_scoped_to_one_kind() {
    let mut handlers = BTreeMap::from([("shared".to_string(), 1)]);
    let mut filters = BTreeMap::from([("shared".to_string(), 2)]);

    // Only the filter identifier "shared" is reachable, so the handler under
    // the same name must still be pruned: the two registries are independent.
    prune_handlers(&mut handlers, &ids(&[], &["shared"]));
    prune_filters(&mut filters, &ids(&[], &["shared"]));

    assert!(
        handlers.is_empty(),
        "a filter identifier must not keep a same-named handler alive",
    );
    assert_eq!(
        filters.len(),
        1,
        "the reachable filter identifier must be retained",
    );
}

#[rstest]
fn unreachable_reports_both_kinds_separately_and_sorted() {
    let supplied = ids(&["h2", "h1"], &["f2", "f1"]);
    let reachable = ids(&["h1"], &[]);

    let (handlers, filters) =
        unreachable_definition_ids(&supplied, &reachable).expect("h2, f1, and f2 are unreachable");

    assert_eq!(
        handlers,
        vec!["h2".to_string()],
        "unreachable handler identifiers must be reported in sorted order",
    );
    assert_eq!(
        filters,
        vec!["f1".to_string(), "f2".to_string()],
        "unreachable filter identifiers must be reported in sorted order",
    );
}

#[rstest]
fn unreachable_is_none_when_everything_is_reachable() {
    let supplied = ids(&["h1"], &["f1"]);
    let reachable = ids(&["h1", "extra"], &["f1"]);

    assert_eq!(
        unreachable_definition_ids(&supplied, &reachable),
        None,
        "a supplied identifier set that the graph covers must not be rejected",
    );
}

#[rstest]
fn unreachable_error_names_each_kind_and_omits_empty_kinds() {
    let handlers_only =
        unreachable_definition_error(&["orphan".to_string()], &[] as &[String]).to_string();
    assert!(
        handlers_only.contains("handlers") && handlers_only.contains("orphan"),
        "the error must name the unreachable handler identifiers, got {handlers_only}",
    );
    assert!(
        !handlers_only.contains("filters"),
        "the error must not mention a kind with no unreachable identifiers, got {handlers_only}",
    );

    let both =
        unreachable_definition_error(&["orphan".to_string()], &["stale".to_string()]).to_string();
    assert!(
        both.contains("handlers") && both.contains("filters"),
        "the error must separate handler and filter identifiers, got {both}",
    );
}

#[rstest]
fn unreachable_error_is_an_invalid_mutation() {
    let err = unreachable_definition_error(&["orphan".to_string()], &[] as &[String]);

    assert!(
        matches!(err, ConfigError::InvalidMutation(_)),
        "an unreferenced definition must be reported as an invalid mutation",
    );
}
