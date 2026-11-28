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

/// What the scan loop observed, used to decide whether a tail probe is valid.
struct ScanState {
    /// Exact length, when the attribute exposed one.
    known_len: Option<usize>,
    /// Whether retention stopped because the ceiling was already full.
    tail_after_retention: bool,
    /// Whether the aggregate budget refused a note.
    exhausted: bool,
    /// Whether the iterator raised part way through.
    iteration_failed: bool,
    /// Whether the loop consumed every probe position.
    iterator_ended: bool,
}

/// Consume one position past the scan and report whether a tail was proven.
///
/// A successful extra item proves the attribute held more than the scan saw.
/// A failing extra item is counted as a conversion failure rather than as an
/// omitted note, because no ceiling has been shown to drop anything.
fn probe_tail(
    state: ScanState,
    iterator: &mut Bound<'_, PyIterator>,
    marker: &mut PayloadTruncation,
) -> bool {
    let ready = state.known_len.is_none()
        && !state.tail_after_retention
        && !state.exhausted
        && !state.iteration_failed
        && state.iterator_ended;
    if !ready {
        return false;
    }
    match iterator.next() {
        None => false,
        // Only a string proves a note was dropped; an unclassified item leaves
        // the tail unknown, which is not reported as an omitted note.
        Some(Ok(item)) => item.cast::<PyString>().is_ok(),
        Some(Err(_)) => {
            marker.failed_elements = marker.failed_elements.saturating_add(1);
            false
        }
    }
}

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
    let probe_limit = limits.max_notes.saturating_add(1);
    let scan_limit = known_len.map_or(probe_limit, |length| length.min(probe_limit));
    let mut retained = Vec::with_capacity(scan_limit.min(limits.max_notes));
    let mut visited = 0usize;
    let mut exhausted = false;
    let mut tail_after_retention = false;
    let mut iterator_ended = false;
    let mut iteration_failed = false;
    let mut iterator = iterator;
    for item in iterator.by_ref().take(scan_limit) {
        visited += 1;
        iterator_ended = visited == scan_limit;
        let item = match item {
            Ok(item) => item,
            Err(_) => {
                iteration_failed = true;
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
    // An unknown-length scan that filled every probe position may have stopped
    // at the work ceiling rather than at the end of the attribute. One extra
    // item distinguishes the two without draining the iterator; when it exists
    // the scan is incomplete, but its size is unknown.
    let completed = ScanState {
        known_len,
        tail_after_retention,
        exhausted,
        iteration_failed,
        iterator_ended,
    };
    let probed_note = probe_tail(completed, &mut iterator, marker);
    // A note is only counted as omitted when a probe proves one was dropped,
    // or when the budget refused a note it had already rendered. Entries past
    // the probe stay unclassified: their raw positions are not evidence that
    // they are notes, so the count is a confirmed minimum rather than an
    // inferred total.
    let omitted = usize::from(tail_after_retention || probed_note || exhausted);
    marker.notes_omitted = marker.notes_omitted.saturating_add(as_u32(omitted));
    (retained, exhausted)
}
