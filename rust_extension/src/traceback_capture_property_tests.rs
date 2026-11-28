//! Property tests for the bounded-capture accounting helpers.
//!
//! These three helpers carry the invariants every capture path relies on:
//! truncation never exceeds its byte ceiling, the aggregate budget never
//! over-spends, and merging markers preserves exact counts and flags. The
//! examples in the unit tests pin the shapes; these properties check the rules
//! over arbitrary inputs, including oversized and multi-byte text.

use proptest::prelude::*;

use crate::exception_schema::PayloadTruncation;
use crate::traceback_capture::{
    CaptureBudget, CaptureLimits, TRUNCATION_SUFFIX, TRUNCATION_SUFFIX_LEN, truncate_to_limit,
};

proptest! {
    #[test]
    fn truncated_text_fits_the_limit_and_stays_valid_utf8(
        text in ".{0,200}",
        limit in 0usize..64,
    ) {
        let Some(retained) = truncate_to_limit(&text, limit) else {
            return Ok(());
        };
        prop_assert!(retained.len() <= limit, "retained {} > {}", retained.len(), limit);
        prop_assert!(std::str::from_utf8(retained.as_bytes()).is_ok());
        if limit >= TRUNCATION_SUFFIX_LEN {
            prop_assert!(retained.ends_with(TRUNCATION_SUFFIX));
        }
    }

    #[test]
    fn a_budget_never_retains_more_than_its_aggregate_allowance(
        operations in prop::collection::vec((0u8..3, 0usize..128, ".{0,40}"), 0..32),
        capacity in 0usize..256,
        nodes in 0usize..16,
    ) {
        let limits = CaptureLimits {
            max_aggregate_bytes: capacity,
            max_nodes: nodes,
            ..CaptureLimits::default()
        };
        let mut budget = CaptureBudget::new(&limits);
        let mut retained = 0usize;
        for (kind, amount, text) in operations {
            match kind {
                0 => {
                    if budget.charge_bytes(amount) {
                        retained += amount;
                    } else {
                        prop_assert!(budget.is_exhausted());
                    }
                }
                1 => {
                    let _ = budget.charge_node();
                }
                _ => {
                    let (kept, _) = budget.retain_text(text);
                    retained += kept.len();
                }
            }
            prop_assert!(retained <= capacity);
            prop_assert!(budget.remaining_bytes() <= capacity);
        }
    }

    #[test]
    fn merging_markers_saturates_counts_and_ors_flags(
        counts_a in prop::array::uniform8(any::<u32>()),
        counts_b in prop::array::uniform8(any::<u32>()),
        flags_a in prop::array::uniform8(any::<bool>()),
        flags_b in prop::array::uniform8(any::<bool>()),
    ) {
        let a = marker_from(counts_a, flags_a);
        let b = marker_from(counts_b, flags_b);
        let mut merged = a;
        merged.merge(&b);

        prop_assert_eq!(merged.args_omitted, a.args_omitted.saturating_add(b.args_omitted));
        prop_assert_eq!(merged.notes_omitted, a.notes_omitted.saturating_add(b.notes_omitted));
        prop_assert_eq!(merged.frames_omitted, a.frames_omitted.saturating_add(b.frames_omitted));
        prop_assert_eq!(
            merged.exceptions_omitted,
            a.exceptions_omitted.saturating_add(b.exceptions_omitted)
        );
        prop_assert_eq!(
            merged.failed_elements,
            a.failed_elements.saturating_add(b.failed_elements)
        );
        prop_assert_eq!(
            merged.message_truncated,
            a.message_truncated || b.message_truncated
        );
        prop_assert_eq!(merged.args_truncated, a.args_truncated || b.args_truncated);
        prop_assert_eq!(merged.notes_truncated, a.notes_truncated || b.notes_truncated);
        prop_assert_eq!(merged.cause_omitted, a.cause_omitted || b.cause_omitted);
        prop_assert_eq!(merged.context_omitted, a.context_omitted || b.context_omitted);
        prop_assert_eq!(
            merged.members_truncated,
            a.members_truncated || b.members_truncated
        );
        prop_assert_eq!(
            merged.budget_exhausted,
            a.budget_exhausted || b.budget_exhausted
        );
    }
}

/// Build a marker from the generated counts and flags.
fn marker_from(counts: [u32; 8], flags: [bool; 8]) -> PayloadTruncation {
    PayloadTruncation {
        message_truncated: flags[0],
        args_omitted: counts[0],
        args_truncated: flags[1],
        notes_omitted: counts[1],
        notes_truncated: flags[2],
        frames_omitted: counts[2],
        frames_omitted_is_lower_bound: flags[3],
        cause_omitted: flags[4],
        context_omitted: flags[5],
        exceptions_omitted: counts[3],
        members_truncated: flags[6],
        failed_elements: counts[4],
        budget_exhausted: flags[7],
    }
}
