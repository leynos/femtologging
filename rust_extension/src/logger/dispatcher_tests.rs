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

/// One logger's budget leaves another logger a slot on any multi-slot shard.
///
/// A budget equal to the depth is no bound at all: on a machine reporting one
/// core the pool's depth is exactly `RECORDS_PER_LOGGER`, so a logger granted
/// the full figure could take every permit and every other logger on that
/// shard would be refused. That is the starvation the budget exists to
/// prevent, which is why the guarantee is asserted here as arithmetic for the
/// shallow cases a test can build rather than left to a deep pool it cannot.
///
/// A one-deep shard is exempt because the property is impossible there rather
/// than violated: it holds a single record, so whoever takes the permit takes
/// the shard. Production never builds one — a pool shard's depth is
/// `RECORDS_PER_LOGGER * shard_count` — and a budget of zero would be worse,
/// refusing every record on a shard that can still hold one.
#[test]
fn a_loggers_budget_leaves_another_logger_a_slot() {
    use super::shard::Shard;

    for depth in 1..=8 {
        let shard = Shard::start(0, depth).expect("a test shard should start");
        let limit = shard.budget().limit_for_test();
        assert!(limit >= 1, "a budget must never be zero on any shard");
        if depth > 1 {
            assert!(
                limit < depth,
                "a budget of {limit} on a {depth}-deep shard would let one \
                 logger take every permit"
            );
        }
        assert!(
            shard.wait_for_drain(),
            "the test shard's worker must stop cleanly"
        );
    }
}
