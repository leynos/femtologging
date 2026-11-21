//! Bounded collection of an exception's `__notes__`.
//!
//! `add_note()` appends to an ordinary list that application code can also
//! assign to directly, so the attribute is untrusted in both size and type: it
//! may be a list of a million strings, a generator that never ends, or an
//! object that is not iterable at all.
//!
//! Every one of those is contained here. A non-iterable attribute is ignored;
//! non-string elements are skipped without being converted, matching the
//! existing partial-extraction rule; and the count and per-note byte ceilings
//! are checked *before* a note is copied or truncated, so an oversized attribute
//! costs the ceilings rather than its own size.

use pyo3::exceptions::PyAttributeError;
use pyo3::prelude::*;
use pyo3::types::{PyIterator, PyString};

use super::attrs::{self, as_u32};
use super::limits::{CaptureBudget, CaptureLimits};
use super::values;
use crate::exception_schema::PayloadTruncation;

/// Collect `__notes__` entries, stopping at the count and byte limits.
///
/// Exact built-in lists and tuples expose their C-level length without invoking
/// user code, so their omitted count is exact without scanning the tail. A
/// subclass may override `__iter__`, so it is probed like any other iterable:
/// through `max_notes + 1` items, reporting one omitted item when that probe
/// shows a tail.
///
/// Returns `(retained, exhausted)`.
pub(super) fn retain_notes(
    exception: &Bound<'_, PyAny>,
    limits: &CaptureLimits,
    budget: &mut CaptureBudget,
    marker: &mut PayloadTruncation,
) -> (Vec<String>, bool) {
    let notes = match exception.getattr("__notes__") {
        Ok(notes) if !notes.is_none() => notes,
        Ok(_) => return (Vec::new(), false),
        Err(error) if error.is_instance_of::<PyAttributeError>(exception.py()) => {
            return (Vec::new(), false);
        }
        Err(_) => {
            marker.failed_elements = marker.failed_elements.saturating_add(1);
            return (Vec::new(), false);
        }
    };
    let known_len = attrs::exact_collection_len(&notes);
    let Ok(iterator) = PyIterator::from_object(&notes) else {
        // A `__notes__` attribute that is not iterable is ignored, keeping the
        // existing degradation contract.
        marker.failed_elements = marker.failed_elements.saturating_add(1);
        return (Vec::new(), false);
    };
    let scan_limit = known_len.map_or_else(
        || limits.max_notes.saturating_add(1),
        |length| length.min(limits.max_notes),
    );
    let mut retained = Vec::with_capacity(scan_limit.min(limits.max_notes));
    let mut visited = 0usize;
    let mut exhausted = false;
    let mut failed = false;
    let mut tail_after_retention = false;
    for item in iterator.take(scan_limit) {
        visited += 1;
        let item = match item {
            Ok(item) => item,
            Err(_) => {
                failed = true;
                marker.failed_elements = marker.failed_elements.saturating_add(1);
                break;
            }
        };
        // Only real strings are notes; other types are skipped silently,
        // matching the existing partial-extraction rule.
        let Ok(note) = item.cast::<PyString>() else {
            continue;
        };
        if retained.len() >= limits.max_notes {
            tail_after_retention = true;
            break;
        }
        let rendered = values::bounded_string(note, limits.max_note_bytes);
        if rendered.failed {
            marker.failed_elements = marker.failed_elements.saturating_add(1);
            continue;
        }
        if !budget.charge_bytes(rendered.text.len()) {
            exhausted = true;
            break;
        }
        // Only a note that survived the charge is part of the payload, so only
        // then can its shortening be reported as a property of retained text.
        marker.notes_truncated |= rendered.truncated;
        retained.push(rendered.text);
    }
    let omitted = known_len.map_or_else(
        || {
            // A tail is only proven when retention stopped because the ceiling
            // was already full. An extra position consumed by a non-note, or
            // by the last retained note, shows no missing note.
            usize::from(tail_after_retention || failed || exhausted)
        },
        |length| {
            length
                .saturating_sub(visited)
                .saturating_add(usize::from(failed || exhausted))
        },
    );
    marker.notes_omitted = marker.notes_omitted.saturating_add(as_u32(omitted));
    (retained, exhausted)
}
