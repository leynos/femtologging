//! Tests for the dispatch pool's pure bound arithmetic.
//!
//! The pool's behaviour is asserted where it is observable — through a logger
//! in `logger_tests.rs` and `logger_tests_shutdown.rs` — so what is left here
//! is the one decision that needs no threads to test: how many shards a machine
//! reporting a given parallelism gets.

use super::shards_for;

/// The default the pool falls back to when the machine reports no limit.
const DEFAULT: usize = super::DEFAULT_SHARD_COUNT;

#[test]
fn a_machine_with_plenty_of_cores_gets_the_default_pool() {
    assert_eq!(
        shards_for(Some(64)),
        DEFAULT,
        "a 64-core machine must not turn into 64 logging threads"
    );
}

#[test]
fn a_small_machine_gets_one_shard_per_core() {
    assert_eq!(shards_for(Some(2)), 2, "two cores can use two shards");
    assert_eq!(shards_for(Some(1)), 1, "one core can use one shard");
    assert_eq!(
        shards_for(Some(DEFAULT)),
        DEFAULT,
        "a machine with exactly the cap's cores must get the cap, not one fewer"
    );
    assert_eq!(
        shards_for(Some(DEFAULT + 1)),
        DEFAULT,
        "one core past the cap must still clamp down to it"
    );
}

#[test]
fn a_machine_reporting_nothing_gets_a_usable_pool() {
    assert_eq!(
        shards_for(None),
        DEFAULT,
        "an unknown machine must still get a pool"
    );
    assert_eq!(
        shards_for(Some(0)),
        1,
        "a machine claiming zero cores must still get one shard, never none"
    );
}
