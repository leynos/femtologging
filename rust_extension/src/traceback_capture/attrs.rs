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
use pyo3::types::{PyIterator, PyList, PyString, PyTuple};

use super::limits::{CaptureLimits, truncate_to_limit};
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
    let type_name = leaf_type_name(&exc_type);
    let module = optional_string_attr(&exc_type, "__module__", limits.max_function_bytes)
        .filter(|name: &String| name != "builtins");

    (
        clamp_identifier(type_name, limits.max_function_bytes),
        module.map(|name| clamp_identifier(name, limits.max_function_bytes)),
    )
}

/// Read a type's leaf name before any clamping happens.
///
/// `__name__` already holds the leaf, so a very long class name cannot lose it
/// to truncation and a cut suffix can never be mistaken for part of a dotted
/// path. The qualname is only split when `__name__` is missing, and that split
/// happens here so the caller's clamp applies to the leaf alone.
fn leaf_type_name(exc_type: &Bound<'_, PyAny>) -> String {
    if let Some(name) = raw_string_attr(exc_type, "__name__") {
        return name;
    }
    let Some(qualname) = raw_string_attr(exc_type, "__qualname__") else {
        return String::new();
    };
    // A nested class reports `Outer.Inner`; the leaf name is the useful part.
    qualname
        .rsplit_once('.')
        .map_or_else(|| qualname.clone(), |(_, name)| name.to_owned())
}

/// Read a string attribute in full, so callers can cut it after inspection.
fn raw_string_attr(obj: &Bound<'_, PyAny>, attr: &str) -> Option<String> {
    let value = optional_attr(obj, attr)?;
    let value = value.cast::<PyString>().ok()?;
    Some(value.to_string_lossy().into_owned())
}

/// Read and clamp an optional string attribute without returning a borrowed
/// PyO3 view from a closure that owns its temporary `Bound` value.
fn optional_string_attr(obj: &Bound<'_, PyAny>, attr: &str, limit: usize) -> Option<String> {
    let value = optional_attr(obj, attr)?;
    let value = value.cast::<PyString>().ok()?;
    Some(values::bounded_string(value, limit).text)
}

/// Shorten an identifier-like field to `limit` bytes.
fn clamp_identifier(value: String, limit: usize) -> String {
    truncate_to_limit(&value, limit).unwrap_or(value)
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
    let known_len = sequence
        .cast::<PyList>()
        .map(|list| list.len())
        .or_else(|_| sequence.cast::<PyTuple>().map(|tuple| tuple.len()))
        .ok();
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
