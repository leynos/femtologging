//! The bounded record of what capture could not retain in full.
//!
//! Capture is bounded — see [`crate::traceback_capture`] — which means a payload
//! is always a *subset* of the exception it describes. A consumer that cannot
//! tell a complete payload from a cut-down one will report a truncated
//! traceback as though it were the whole story, so every omission is recorded
//! here.
//!
//! The record is deliberately fixed-size. Counts and flags cost the same
//! whatever was discarded, which is what lets the marker describe a
//! one-megabyte message without the description itself needing a budget.
//!
//! # Two families of field
//!
//! - **Counts of omitted items** — `args_omitted`, `notes_omitted`,
//!   `frames_omitted`, `exceptions_omitted`, `failed_elements`. These say how
//!   many things were dropped, so a reader can tell "nothing here" from
//!   "nothing retained".
//! - **Flags for content retained but shortened** — `message_truncated`,
//!   `args_truncated`, `notes_truncated`, `members_truncated`. These say the
//!   text present is a prefix, and carry no count because the exact original
//!   length is not available without measuring the value that was too large to
//!   measure.
//!
//! `budget_exhausted` is the one field that describes the capture as a whole
//! rather than the node it sits on. It means every count in the payload is a
//! lower bound, which is why it is attached only to the root payload.
//!
//! A payload carries a marker only when at least one field is non-default, so
//! consumers must read an absent marker as "nothing was omitted" rather than as
//! "this payload predates the marker". Both readings are the same thing: a
//! payload from before the marker existed also omitted nothing it knew of.

use serde::{Deserialize, Serialize};

/// A bounded record of the components capture could not retain in full.
///
/// Every field is a count or a flag, so the marker has a fixed maximum size no
/// matter how large the discarded data was. A payload carries a truncation
/// marker only when at least one field is non-default; consumers must therefore
/// treat an absent marker as "nothing was omitted".
///
/// Consumers distinguish two families of field: counts of *omitted* items
/// (`args_omitted`, `notes_omitted`, `frames_omitted`, `exceptions_omitted`,
/// `failed_elements`) and flags for content that was retained but shortened
/// (`message_truncated`, `args_truncated`, `notes_truncated`,
/// `members_truncated`).
///
/// # Examples
///
/// ```rust
/// use _femtologging_rs::exception_schema::PayloadTruncation;
///
/// let marker = PayloadTruncation {
///     frames_omitted: 3,
///     ..Default::default()
/// };
/// assert!(!marker.is_empty());
/// assert!(PayloadTruncation::default().is_empty());
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayloadTruncation {
    /// The exception message was shortened to fit its byte ceiling.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub message_truncated: bool,
    /// Number of exception arguments dropped because the count ceiling was hit.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub args_omitted: u32,
    /// One or more retained argument representations were shortened.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub args_truncated: bool,
    /// Number of notes dropped because the count ceiling was hit.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub notes_omitted: u32,
    /// One or more retained notes were shortened.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub notes_truncated: bool,
    /// Number of frames dropped from the innermost edge of this traceback.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub frames_omitted: u32,
    /// The exception cause was dropped because a traversal ceiling was hit.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cause_omitted: bool,
    /// The exception context was dropped because a traversal ceiling was hit.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub context_omitted: bool,
    /// Number of exception-group children dropped.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub exceptions_omitted: u32,
    /// One or more retained group members were themselves truncated.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub members_truncated: bool,
    /// Number of elements skipped because converting them raised.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub failed_elements: u32,
    /// The aggregate traversal budget ran out before capture finished.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub budget_exhausted: bool,
}

/// Serde predicate that omits a counter when it holds no omissions.
fn is_zero(value: &u32) -> bool {
    *value == 0
}

impl PayloadTruncation {
    /// Return `true` when nothing was truncated or omitted.
    ///
    /// Payloads with an empty marker should not carry one at all; this
    /// predicate exists so constructors and consumers agree on that rule.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use _femtologging_rs::exception_schema::PayloadTruncation;
    ///
    /// assert!(PayloadTruncation::default().is_empty());
    /// let marker = PayloadTruncation { args_omitted: 1, ..Default::default() };
    /// assert!(!marker.is_empty());
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Return `self` as `Some` unless every field is default.
    ///
    /// Walkers accumulate into a marker directly, then call this once at the
    /// end so that a payload only carries a marker when it is meaningful.
    #[must_use]
    pub fn into_option(self) -> Option<Self> {
        (!self.is_empty()).then_some(self)
    }

    /// Fold `other` into `self`, merging counts and OR-ing flags.
    ///
    /// Group members report their own omissions to a parent that also has
    /// omissions of its own, so the parent has to combine both. Counters add
    /// rather than saturate so that `members_truncated` stays meaningful.
    pub fn merge(&mut self, other: &Self) {
        self.message_truncated |= other.message_truncated;
        self.args_omitted = self.args_omitted.saturating_add(other.args_omitted);
        self.args_truncated |= other.args_truncated;
        self.notes_omitted = self.notes_omitted.saturating_add(other.notes_omitted);
        self.notes_truncated |= other.notes_truncated;
        self.frames_omitted = self.frames_omitted.saturating_add(other.frames_omitted);
        self.cause_omitted |= other.cause_omitted;
        self.context_omitted |= other.context_omitted;
        self.exceptions_omitted = self
            .exceptions_omitted
            .saturating_add(other.exceptions_omitted);
        self.members_truncated |= other.members_truncated;
        self.failed_elements = self.failed_elements.saturating_add(other.failed_elements);
        self.budget_exhausted |= other.budget_exhausted;
    }
}
