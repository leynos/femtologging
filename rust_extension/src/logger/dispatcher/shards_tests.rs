//! Slot lookup and construction for a pool's shard list.
//!
//! `Shards` keeps the first shard apart from the rest so an empty pool is not
//! representable, and its slot lookup falls back to that first shard rather
//! than panicking. Both behaviours are structural, so they are asserted here
//! instead of being left to the assignment property tests, which exercise them
//! only through round robin.

use std::io::ErrorKind;
use std::ptr;

use super::Shards;
use crate::logger::dispatcher::shard::Shard;

// `Shards::len` is `const`, and the assignment path relies on that: the
// round-robin wraparound is arithmetic a caller can fold. A `const fn` that
// quietly loses its qualifier still passes every behavioural test, so the
// guarantee is pinned at compile time.
const _: fn(&Shards) -> usize = Shards::len;

/// Build a pool of `count` shards, asserting each worker started.
fn pool_of(count: usize) -> Shards {
    let started = (0..count)
        .map(|index| {
            Shard::start(index, 8).unwrap_or_else(|error| {
                panic!("shard {index} of {count} should start, got {error}")
            })
        })
        .collect();
    Shards::from_started(started).expect("a pool of at least one shard is valid")
}

/// A pool reports the number of shards it was built from.
///
/// The count is one more than `rest`, because the first shard is stored apart
/// from it. An off-by-one here would make `assign_shard`'s modulo skip or
/// repeat a shard, so the arithmetic is asserted directly for the smallest
/// pools a machine can produce.
#[test]
fn a_pool_reports_the_number_of_shards_it_was_built_from() {
    for count in 1..=4 {
        assert_eq!(
            pool_of(count).len(),
            count,
            "a pool built from {count} shards must report {count}",
        );
    }
}

/// Slot zero and every slot past the end answer with the first shard.
///
/// The fallback is what makes assignment total after the round-robin counter
/// wraps: a slot is derived from `len`, but a lookup must still answer rather
/// than panic if it ever names a shard the pool does not hold. Comparing
/// pointers is the assertion that the *same* shard comes back, not merely one
/// with equal fields.
#[test]
fn slot_zero_and_slots_past_the_end_answer_with_the_first_shard() {
    let shards = pool_of(3);

    assert!(
        ptr::eq(shards.get(0), shards.get(shards.len())),
        "slot zero and the first slot past the end must name one shard",
    );
    for slot in [3, 4, usize::MAX] {
        assert!(
            ptr::eq(shards.get(0), shards.get(slot)),
            "slot {slot} past a pool of {} must fall back to the first shard",
            shards.len(),
        );
    }
}

/// Every slot inside the pool answers with the shard started for it.
///
/// Round robin assigns by index, so a lookup that returned the wrong shard
/// would silently unbalance the pool: one worker would take two loggers'
/// records while another took none. The order is start order, and each slot is
/// asserted distinct from its neighbours so a broken `rest` index cannot pass.
#[test]
fn every_slot_inside_the_pool_answers_with_its_own_shard() {
    let shards = pool_of(4);

    let slots: Vec<&Shard> = (0..shards.len()).map(|slot| shards.get(slot)).collect();
    for (index, shard) in slots.iter().enumerate() {
        for (other_index, other) in slots.iter().enumerate() {
            assert!(
                index == other_index || !ptr::eq(*shard, *other),
                "slots {index} and {other_index} must name different shards",
            );
        }
    }
}

/// A pool built from no shard is refused as an invalid request.
///
/// The refusal is what lets `Dispatcher::assign_shard` return a shard rather
/// than an option: by the time a pool exists it holds one. The error is an
/// invalid request rather than a resource refusal, because nothing was
/// attempted and retrying the same request cannot succeed.
#[test]
fn an_empty_pool_is_refused_as_an_invalid_request() {
    let Err(error) = Shards::from_started(Vec::new()) else {
        panic!("a pool with no shard must be refused");
    };

    assert_eq!(
        error.source.kind(),
        ErrorKind::InvalidInput,
        "an empty pool is an invalid request, not a refusal to start a thread",
    );
    assert!(
        error.to_string().contains("at least one shard"),
        "the refusal must name the invariant: {error}",
    );
}
