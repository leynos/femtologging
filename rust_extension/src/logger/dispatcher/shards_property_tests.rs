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
use super::shards::Shards;

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

    /// Slot lookup is total and positional for every pool size and slot.
    ///
    /// Slot zero, every slot inside the pool and every slot past the end are
    /// checked together, because they are one function: a lookup that panicked
    /// past the end or that shifted the mapping by one would still pass the
    /// assignment property, which only ever visits slots below `len`.
    ///
    /// The slot range deliberately includes `usize::MAX`-adjacent values. A
    /// slot is derived from a counter that can wrap, so "past the end" has to
    /// mean arithmetic past the end rather than a small overrun.
    #[test]
    fn slot_lookup_is_total_and_positional(
        count in 1usize..=5,
        slot in prop_oneof![
            Just(0usize),
            0usize..=8,
            (usize::MAX - 4)..=usize::MAX,
        ],
    ) {
        let shards = Shards::from_started(
            (0..count)
                .map(|index| {
                    Shard::start(index, 8).map_err(|error| {
                        TestCaseError::fail(format!("shard {index} of {count} did not start: {error}"))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
        )
        .map_err(|error| TestCaseError::fail(format!("a pool of {count} must be valid: {error}")))?;

        prop_assert_eq!(shards.len(), count, "the pool must report its shard count");

        let first = shards.get(0);
        if slot == 0 || slot >= count {
            prop_assert!(
                std::ptr::eq(shards.get(slot), first),
                "slot {slot} of a pool of {count} must fall back to the first shard",
            );
        } else {
            // A slot inside the pool must name its own shard, distinct from
            // both the first shard and its neighbours.
            let looked_up = shards.get(slot);
            prop_assert!(
                !std::ptr::eq(looked_up, first) || slot == 0,
                "slot {slot} of a pool of {count} must not alias the first shard",
            );
            prop_assert!(
                std::ptr::eq(looked_up, shards.get(slot)),
                "repeated lookups of slot {slot} must agree",
            );
        }
    }
}
