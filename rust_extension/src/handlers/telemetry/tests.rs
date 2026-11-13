//! Unit tests for the counter table, snapshots, and the wait timer.

use super::diagnostics::WaitDiagnostics;
use super::outcome::{OUTCOME_COUNT, WaitOutcome};
use super::timer::{WaitContext, WaitTimer};
use std::collections::HashSet;
use std::time::Duration;

/// Record one outcome with an explicit duration, bypassing the clock.
fn record_with(diagnostics: &WaitDiagnostics, outcome: WaitOutcome, micros: u64) {
    diagnostics.record(outcome, Duration::from_micros(micros));
}

#[test]
fn every_outcome_has_a_unique_key() {
    let keys: HashSet<&'static str> = WaitOutcome::ALL.iter().map(|o| o.key()).collect();
    assert_eq!(keys.len(), OUTCOME_COUNT, "outcome keys must be unique");
}

#[test]
fn the_exposed_map_has_a_fixed_key_set() {
    let map = WaitDiagnostics::new().snapshot().to_map();
    assert_eq!(
        map.len(),
        OUTCOME_COUNT + 3,
        "counters, two totals, and the in-flight gauge"
    );
    for outcome in WaitOutcome::ALL {
        assert!(map.contains_key(outcome.key()), "missing {}", outcome.key());
    }
    assert!(map.contains_key("total_wait_micros"));
    assert!(map.contains_key("max_wait_micros"));
    assert!(map.contains_key("waits_in_flight"));
}

#[test]
fn the_in_flight_gauge_tracks_the_guard() {
    let diagnostics = WaitDiagnostics::new();
    assert_eq!(diagnostics.snapshot().in_flight(), 0);

    let first = diagnostics.begin_wait();
    let second = diagnostics.begin_wait();
    assert_eq!(
        diagnostics.snapshot().in_flight(),
        2,
        "both guards must be visible while they are held"
    );

    drop(first);
    assert_eq!(diagnostics.snapshot().in_flight(), 1);
    drop(second);
    assert_eq!(
        diagnostics.snapshot().in_flight(),
        0,
        "dropping every guard must clear the gauge"
    );
}

#[test]
fn recording_increments_only_the_named_counter() {
    let diagnostics = WaitDiagnostics::new();
    record_with(&diagnostics, WaitOutcome::HandleAccepted, 10);
    record_with(&diagnostics, WaitOutcome::HandleAccepted, 10);
    record_with(&diagnostics, WaitOutcome::FlushAckTimedOut, 10);
    let snapshot = diagnostics.snapshot();

    assert_eq!(snapshot.count(WaitOutcome::HandleAccepted), 2);
    assert_eq!(snapshot.count(WaitOutcome::FlushAckTimedOut), 1);
    assert_eq!(snapshot.count(WaitOutcome::CloseCompleted), 0);
}

#[test]
fn durations_accumulate_and_track_the_longest_wait() {
    let diagnostics = WaitDiagnostics::new();
    record_with(&diagnostics, WaitOutcome::HandleAccepted, 1_000);
    record_with(&diagnostics, WaitOutcome::CloseCompleted, 5_000);
    let snapshot = diagnostics.snapshot();

    assert_eq!(snapshot.total_wait_micros(), 6_000);
    assert_eq!(snapshot.max_wait_micros(), 5_000);
    assert!(snapshot.total_wait_micros() >= snapshot.max_wait_micros());
}

#[test]
fn the_timer_records_exactly_one_wait() {
    let diagnostics = WaitDiagnostics::new();
    let timer = WaitTimer::start();
    timer.finish(
        &diagnostics,
        WaitContext {
            operation: "handler_flush",
            handler_kind: "file",
            policy_bucket: "drop",
        },
        WaitOutcome::FlushAcknowledged,
    );
    let snapshot = diagnostics.snapshot();
    assert_eq!(snapshot.count(WaitOutcome::FlushAcknowledged), 1);
    assert_eq!(snapshot.total_wait_micros(), snapshot.max_wait_micros());
}

#[test]
fn outcomes_index_the_counter_array_without_gaps() {
    // `WaitOutcome` is cast to `usize`, so a gap would silently alias two
    // outcomes onto one counter.
    for (index, outcome) in WaitOutcome::ALL.iter().enumerate() {
        assert_eq!(
            outcome.index(),
            index,
            "{} must sit at {index}",
            outcome.key()
        );
    }
}
