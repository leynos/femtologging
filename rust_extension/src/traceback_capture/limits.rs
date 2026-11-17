//! Resource limits for bounded exception and stack capture.
//!
//! Exception payloads are built on the caller thread while the GIL is held, so
//! every traversal must have a predictable upper bound. [`CaptureLimits`]
//! carries the per-field ceilings that shape a payload; [`CaptureBudget`]
//! tracks the shared aggregate allowance that every field draws from.
//!
//! # Budget accounting
//!
//! Only retained UTF-8 bytes and retained nodes are counted, never work that is
//! discarded. Callers therefore charge the budget *after* deciding what to keep,
//! which keeps the accounting identical for payloads of the same shape
//! regardless of how many oversized elements were offered to the walker.
//!
//! # Budget order
//!
//! Fields are charged in a fixed order — type, message, frames, args, notes,
//! cause, context, group members — so that exhausting the aggregate budget
//! always truncates the least important data first.

/// Marker appended to a value that was cut short.
///
/// The marker is ASCII so that a truncated string stays valid UTF-8 whatever
/// the input encoding was, and its length is included in the per-field ceiling
/// rather than being added on top of it.
pub const TRUNCATION_SUFFIX: &str = "...";

/// Size of [`TRUNCATION_SUFFIX`] in bytes.
pub const TRUNCATION_SUFFIX_LEN: usize = TRUNCATION_SUFFIX.len();

/// Placeholder message used when stringifying the exception itself fails.
///
/// Python's own `traceback` module uses an identical sentinel, so capture
/// output stays recognisable across the two implementations.
pub const STR_FAILURE_SENTINEL: &str = "<exception str() failed>";

/// Per-field ceilings and aggregate allowances for one capture.
///
/// Defaults are deliberately conservative: an ordinary exception is captured in
/// full, while a hostile or pathological one is cut down to a payload that a
/// downstream formatter can render without further bounds.
///
/// # Examples
///
/// ```rust,ignore
/// let limits = CaptureLimits::default();
/// let mut budget = CaptureBudget::new(&limits);
/// assert!(limits.max_message_bytes > 0);
/// assert!(budget.charge_node());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureLimits {
    /// Maximum retained UTF-8 bytes for the exception message.
    pub max_message_bytes: usize,
    /// Maximum number of exception arguments whose representations are kept.
    pub max_args: usize,
    /// Maximum retained UTF-8 bytes for a single argument representation.
    pub max_arg_bytes: usize,
    /// Maximum number of exception notes kept.
    pub max_notes: usize,
    /// Maximum retained UTF-8 bytes for a single note.
    pub max_note_bytes: usize,
    /// Maximum number of frames retained per traceback or stack.
    pub max_frames: usize,
    /// Maximum retained UTF-8 bytes for a frame's source line.
    pub max_source_line_bytes: usize,
    /// Maximum retained UTF-8 bytes for a frame's filename.
    pub max_filename_bytes: usize,
    /// Maximum retained UTF-8 bytes for a frame's function name.
    pub max_function_bytes: usize,
    /// Maximum number of cause/context links followed from the root.
    pub max_chain_depth: usize,
    /// Maximum number of nested exception groups descended into.
    pub max_group_depth: usize,
    /// Maximum number of children retained from one exception group.
    pub max_group_width: usize,
    /// Maximum number of exception nodes visited across the whole traversal.
    pub max_nodes: usize,
    /// Maximum aggregate retained UTF-8 bytes across the whole traversal.
    pub max_aggregate_bytes: usize,
}

impl Default for CaptureLimits {
    fn default() -> Self {
        Self {
            max_message_bytes: 4 * 1024,
            max_args: 32,
            max_arg_bytes: 512,
            max_notes: 32,
            max_note_bytes: 512,
            max_frames: 64,
            max_source_line_bytes: 512,
            max_filename_bytes: 512,
            max_function_bytes: 256,
            max_chain_depth: 32,
            max_group_depth: 10,
            max_group_width: 15,
            max_nodes: 256,
            max_aggregate_bytes: 256 * 1024,
        }
    }
}

/// Running allowance shared by every field of one capture.
///
/// The budget is charged in retained units only. A field that finds the budget
/// exhausted records that fact rather than growing the payload.
///
/// # Examples
///
/// ```rust,ignore
/// let limits = CaptureLimits::default();
/// let mut budget = CaptureBudget::new(&limits);
/// assert!(budget.charge_node());
/// assert!(budget.charge_bytes(8));
/// ```
#[derive(Debug)]
pub struct CaptureBudget {
    /// Node slots left for exception and group traversal.
    remaining_nodes: usize,
    /// UTF-8 bytes left for retained payload text.
    remaining_bytes: usize,
    /// Whether any allowance has been exhausted.
    exhausted: bool,
}

impl CaptureBudget {
    /// Create a fresh budget from `limits`.
    #[must_use]
    pub fn new(limits: &CaptureLimits) -> Self {
        Self {
            remaining_nodes: limits.max_nodes,
            remaining_bytes: limits.max_aggregate_bytes,
            exhausted: false,
        }
    }

    /// Claim one node from the traversal allowance.
    ///
    /// Returns `false` when the node allowance is spent, in which case the
    /// caller must stop descending and record the exhaustion.
    pub fn charge_node(&mut self) -> bool {
        if self.remaining_nodes == 0 {
            self.exhausted = true;
            return false;
        }
        self.remaining_nodes -= 1;
        true
    }

    /// Return `true` when `bytes` still fit inside the aggregate allowance.
    #[must_use]
    pub fn has_capacity_for(&self, bytes: usize) -> bool {
        bytes <= self.remaining_bytes
    }

    /// Charge `bytes` of retained data.
    ///
    /// Returns `false` and marks the budget exhausted when the allowance is
    /// insufficient, leaving the budget unchanged so callers can fall back to
    /// omitting the field entirely.
    pub fn charge_bytes(&mut self, bytes: usize) -> bool {
        if !self.has_capacity_for(bytes) {
            self.exhausted = true;
            return false;
        }
        self.remaining_bytes -= bytes;
        true
    }

    /// Return the number of UTF-8 bytes still available for retained text.
    #[must_use]
    pub fn remaining_bytes(&self) -> usize {
        self.remaining_bytes
    }

    /// Retain `value` within the remaining aggregate byte allowance.
    ///
    /// When the full value does not fit, its prefix and truncation suffix are
    /// retained if the remaining allowance can fit them. The budget is charged
    /// only for bytes actually kept. Returns the retained text and whether it
    /// was shortened by the aggregate ceiling.
    pub fn retain_text(&mut self, value: String) -> (String, bool) {
        let shortened = truncate_to_limit(&value, self.remaining_bytes);
        let (retained, was_shortened) = shortened.map_or((value, false), |text| (text, true));
        let charged = self.charge_bytes(retained.len());
        debug_assert!(charged, "retained text must fit the remaining budget");
        if was_shortened {
            self.exhausted = true;
        }
        (retained, was_shortened)
    }

    /// Return `true` if any traversal step ran out of allowance.
    #[must_use]
    pub fn is_exhausted(&self) -> bool {
        self.exhausted
    }
}

/// Truncate `value` so that it fits within `limit` bytes of UTF-8.
///
/// The ASCII [`TRUNCATION_SUFFIX`] is counted *inside* `limit`, so the returned
/// string never exceeds the requested size in bytes. Cutting happens on a
/// character boundary, which keeps the result valid UTF-8 for any input.
///
/// Returns `None` when `value` already fits and no marker is needed.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(truncate_to_limit("short", 16), None);
/// assert_eq!(truncate_to_limit("abcdefgh", 5).as_deref(), Some("ab..."));
/// // Multi-byte characters are cut on a boundary, never mid-codepoint.
/// assert_eq!(truncate_to_limit("αααα", 6).as_deref(), Some("α..."));
/// ```
#[must_use]
pub fn truncate_to_limit(value: &str, limit: usize) -> Option<String> {
    if value.len() <= limit {
        return None;
    }
    // A tiny injected test limit may be shorter than the marker itself. Keep
    // the result inside that ceiling; the separate truncation field still
    // reports what could not fit.
    let suffix_bytes = if limit >= TRUNCATION_SUFFIX_LEN {
        TRUNCATION_SUFFIX_LEN
    } else {
        0
    };
    let available = limit.saturating_sub(suffix_bytes);
    let mut cut = available.min(value.len());
    while cut > 0 && !value.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut truncated = String::with_capacity(cut + suffix_bytes);
    truncated.push_str(&value[..cut]);
    truncated.push_str(&TRUNCATION_SUFFIX[..suffix_bytes]);
    Some(truncated)
}
