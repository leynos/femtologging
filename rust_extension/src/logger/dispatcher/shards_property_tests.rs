//! Property tests for round-robin shard assignment.
//!
//! `Shards` keeps the first shard apart from the rest, so a pool of zero shards
//! is not representable, and `Dispatcher::assign_shard` reduces its counter
//! modulo the pool size. Those two together make assignment total and fair.
//! The lifecycle suite pins the shallow fixed cases; these properties check the
//! same invariant over every pool size a small machine would use.

use proptest::prelude::*;

use super::Dispatcher;
use super::shard::Shard;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(12))]

    /// The first `count` assignments cover the pool exactly once, and the
    /// sequence then repeats with period `count`.
    #[test]
    fn assignment_visits_every_shard_before_repeating(count in 1usize..=3) {
        let dispatcher = Dispatcher::start_with(std::process::id(), count, Shard::start)
            .expect("a pool of one to three shards must start");

        let first_round: Vec<&Shard> =
            (0..count).map(|_| dispatcher.assign_shard()).collect();
        for (index, shard) in first_round.iter().enumerate() {
            for (other_index, other) in first_round.iter().enumerate() {
                prop_assert!(
                    index == other_index || !std::ptr::eq(*shard, *other),
                    "a pool of {count} handed out one shard twice within its first \
                     {count} assignments"
                );
            }
        }

        let second_round: Vec<&Shard> =
            (0..count).map(|_| dispatcher.assign_shard()).collect();
        for (first, second) in first_round.iter().zip(second_round.iter()) {
            prop_assert!(
                std::ptr::eq(*first, *second),
                "assignment must repeat with period {count}"
            );
        }
    }
}
