//! The Python-attribute surface the bounded walker reads.
//!
//! Every read here goes through a public Python API and treats a missing,
//! `None` or wrongly-typed value as *absent* rather than as a failure. That is
//! the posture the whole capture rests on: the objects being read come from
//! application code, so a `__notes__` that is not iterable, an exception whose
//! `__module__` was deleted, or a synthetic code object with no `__qualname__`
//! must all degrade into an omitted field instead of an error.
//!
//! The reads are also where captured identifiers get clamped. A type name or
//! module name is attacker-influenceable — a class can be named anything — so it
//! is shortened to the identifier ceiling like any other retained text.
//!
//! [`Link`] lives here because it is the same concern seen from the other side:
//! it names the attribute a chained-exception edge is read from, and records
//! which marker flag reports that edge being dropped.

use pyo3::prelude::*;
use pyo3::types::{PyIterator, PyList, PySlice, PyString, PyTuple};

use super::limits::CaptureLimits;
use super::values;
use crate::exception_schema::PayloadTruncation;

/// Which chained-exception attribute a link refers to.
#[derive(Clone, Copy)]
pub(super) enum Link {
    /// `__cause__`, set by `raise ... from ...`.
    Cause,
    /// `__context__`, set implicitly while handling another exception.
    Context,
}

impl Link {
    /// The attribute this link is read from.
    pub(super) const fn attribute(self) -> &'static str {
        match self {
            Self::Cause => "__cause__",
            Self::Context => "__context__",
        }
    }

    /// Record that a link of this kind was dropped.
    pub(super) fn mark_omitted(self, marker: &mut PayloadTruncation) {
        match self {
            Self::Cause => marker.cause_omitted = true,
            Self::Context => marker.context_omitted = true,
        }
    }

    /// Report whether this link is the exception the cause edge already holds.
    ///
    /// Identity, not equality: a distinct exception with equal contents is a
    /// real context and still worth capturing. Only a context link can
    /// duplicate the cause; the cause edge itself is never redundant.
    ///
    /// The comparison is against the object the cause edge *retained*, not
    /// against a fresh read of `__cause__`. Between the two reads a property can
    /// replace the cause, and comparing the replacement with itself would drop a
    /// context that the payload does not actually contain.
    pub(super) fn is_duplicate_context(
        self,
        retained_cause: Option<&Bound<'_, PyAny>>,
        linked: &Bound<'_, PyAny>,
    ) -> bool {
        matches!(self, Self::Context) && retained_cause.is_some_and(|cause| cause.is(linked))
    }
}

/// Read the exception's type name and module, clamped to the identifier limit.
///
/// A built-in exception's module (`"builtins"`) is elided because it carries no
/// information for a reader or a downstream grouper.
pub(super) fn type_identity(
    exception: &Bound<'_, PyAny>,
    limits: &CaptureLimits,
) -> (String, Option<String>) {
    let exc_type = exception.get_type();
    (
        leaf_type_name(&exc_type, limits.max_function_bytes),
        module_name(&exc_type, limits.max_function_bytes),
    )
}

/// Read a type's module name, dropping the uninformative `builtins` value.
///
/// The comparison happens on the Python string, before the module name is
/// clamped: a three-byte ceiling would otherwise turn `builtins` into `...` and
/// defeat the elision.
fn module_name(obj: &Bound<'_, PyAny>, limit: usize) -> Option<String> {
    let value = string_attr(obj, "__module__")?;
    if value.eq("builtins") {
        return None;
    }
    Some(values::bounded_string(&value, limit).text)
}

/// Read a type's leaf name, bounded to `limit` bytes.
///
/// `__name__` already holds the leaf, so its bounded prefix is the retained
/// name and a very long class name costs the ceiling rather than its own size.
/// The qualname is only consulted when `__name__` is missing, and the leaf is
/// selected with the native `str` method so a dotted path is split before the
/// byte ceiling can remove the part worth keeping.
fn leaf_type_name(exc_type: &Bound<'_, PyAny>, limit: usize) -> String {
    if let Some(name) = string_attr(exc_type, "__name__") {
        return values::bounded_string(&name, limit).text;
    }
    let Some(qualname) = string_attr(exc_type, "__qualname__") else {
        return String::new();
    };
    bounded_leaf(&qualname, limit).unwrap_or_else(|| "<type name unavailable>".to_string())
}

/// Longest qualname scanned when locating its leaf.
///
/// `rsplit` copies every component before the byte ceiling applies, so a
/// megabyte-long qualname would be copied in full on the logging thread.
/// Locating the leaf here scans without copying and only slices the bounded
/// leaf; past this ceiling the scan itself is skipped.
const MAX_QUALNAME_SCAN: usize = 4096;

/// Return the bounded leaf of a dotted qualified name.
///
/// The split is located with the native `str.rfind`, which scans without
/// allocating, and only `limit` code points are then copied. Calling `rsplit`
/// on the instance would run user code for a subclass, so the type's own method
/// is used; a qualname too long to scan yields `None`.
fn bounded_leaf(value: &Bound<'_, PyString>, limit: usize) -> Option<String> {
    let py = value.py();
    let code_points = value.code_point_len().ok()?;
    if code_points > MAX_QUALNAME_SCAN {
        return None;
    }
    let rfind = py.get_type::<PyString>().getattr("rfind").ok()?;
    let last_dot: isize = rfind.call1((value, ".")).ok()?.extract().ok()?;
    let start = last_dot.saturating_add(1).max(0);
    let stop = start.saturating_add(isize::try_from(limit).unwrap_or(isize::MAX));
    let getitem = py.get_type::<PyString>().getattr("__getitem__").ok()?;
    let slice = PySlice::new(py, start, stop, 1);
    let leaf = getitem
        .call1((value, slice))
        .ok()?
        .cast_into::<PyString>()
        .ok()?;
    Some(values::bounded_string(&leaf, limit).text)
}

/// Read a string attribute, without converting it to Rust.
fn string_attr<'py>(obj: &Bound<'py, PyAny>, attr: &str) -> Option<Bound<'py, PyString>> {
    optional_attr(obj, attr)?.cast_into::<PyString>().ok()
}

/// Read an attribute, treating a missing attribute or `None` as absent.
pub(super) fn optional_attr<'py>(obj: &Bound<'py, PyAny>, attr: &str) -> Option<Bound<'py, PyAny>> {
    obj.getattr(attr).ok().filter(|value| !value.is_none())
}

/// Read a boolean attribute, reporting `None` when it is missing or not a bool.
pub(super) fn bool_attr(obj: &Bound<'_, PyAny>, attr: &str) -> Option<bool> {
    optional_attr(obj, attr)?.extract().ok()
}

/// The identity CPython's own cycle detection compares.
pub(super) fn identity(obj: &Bound<'_, PyAny>) -> usize {
    obj.as_ptr() as usize
}

/// Convert a count into the marker's `u32` range.
pub(super) fn as_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// A bounded prefix of a Python iterable and the tail omitted from it.
pub(super) struct BoundedSequence<'py> {
    /// Items retained from the iterable's prefix.
    pub items: Vec<Bound<'py, PyAny>>,
    /// Exact omitted item count for a built-in list or tuple; one means a
    /// generic iterable was found to have a tail.
    pub omitted: usize,
    /// Whether iteration failed after the iterable had been accepted.
    pub failed: bool,
}

/// Read no more than `limit` items, plus one probe for generic iterables.
pub(super) fn bounded_sequence<'py>(
    sequence: &Bound<'py, PyAny>,
    limit: usize,
) -> BoundedSequence<'py> {
    let known_len = exact_collection_len(sequence);
    let scan_limit = known_len.map_or_else(|| limit.saturating_add(1), |len| len.min(limit));
    let mut items = Vec::with_capacity(scan_limit.min(limit));
    let mut failed = false;
    let mut extra_found = false;
    let mut iterator = match PyIterator::from_object(sequence) {
        Ok(iterator) => iterator,
        Err(_) => {
            return BoundedSequence {
                items,
                omitted: known_len.unwrap_or(1),
                failed: true,
            };
        }
    };

    for index in 0..scan_limit {
        match iterator.next() {
            Some(Ok(item)) if index < limit => items.push(item),
            Some(Ok(_)) => {
                extra_found = true;
                break;
            }
            Some(Err(_)) => {
                failed = true;
                break;
            }
            None => break,
        }
    }
    let omitted = known_len.map_or_else(
        || if failed || extra_found { 1 } else { 0 },
        |length| length.saturating_sub(items.len()),
    );
    BoundedSequence {
        items,
        omitted,
        failed,
    }
}

/// Return the C-level length of an exact built-in list or tuple.
///
/// Only the exact types qualify. A subclass can override `__iter__`, so its
/// container length need not describe what iteration yields; treating it as
/// exact would report an omitted count that contradicts the items scanned.
pub(super) fn exact_collection_len(collection: &Bound<'_, PyAny>) -> Option<usize> {
    if collection.is_exact_instance_of::<PyList>() {
        return collection.cast::<PyList>().ok().map(|list| list.len());
    }
    if collection.is_exact_instance_of::<PyTuple>() {
        return collection.cast::<PyTuple>().ok().map(|tuple| tuple.len());
    }
    None
}
