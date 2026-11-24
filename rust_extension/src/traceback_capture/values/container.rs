//! Bounded rendering of exact built-in containers.
//!
//! `repr` of a list, tuple, dict, set or frozenset renders every element and
//! builds the whole text before any byte ceiling can apply. One argument holding
//! a million entries, or holding an exception group whose own `repr` reproduces
//! a shared subtree, would then cost far more than the capture budget on the
//! logging thread.
//!
//! Each container is therefore snapshotted into a bounded list of entries, and
//! then rendered element by element, stopping at explicit item, depth and byte
//! limits. Snapshotting first means no element hook runs while the container
//! itself is being iterated, so an element that mutates its own container
//! cannot invalidate the walk. A container that would exceed a limit is
//! summarised with the truncation marker instead.
//!
//! Subclasses are excluded: they may override `__repr__`, and their own
//! representation is what the caller expects to see.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyFrozenSet, PyIterator, PyList, PySet, PyTuple};

use crate::traceback_capture::limits::TRUNCATION_SUFFIX;
use crate::traceback_capture::values::{BoundedText, clamp, repr_bounded_at};

/// Largest number of container entries rendered before summarising.
const MAX_CONTAINER_ITEMS: usize = 16;

/// Deepest nesting of built-in containers rendered inline.
const MAX_CONTAINER_DEPTH: usize = 4;

/// Render a container, summarising one that exceeds the item, depth or byte ceiling.
///
/// Returns `None` for anything that is not an exact `list`, `tuple`, `dict`,
/// `set` or `frozenset`, leaving the caller to take the general `repr` path.
/// `depth` is the container nesting already rendered above this value.
///
/// A container whose own iteration raises still produces a summary rather than
/// `None`, so the caller never restarts the conversion through native `repr`.
pub(super) fn render(value: &Bound<'_, PyAny>, limit: usize, depth: usize) -> Option<BoundedText> {
    let kind = container_kind(value)?;
    if exact_len(value, kind) == Some(0) {
        return Some(clamp(empty_form(kind).to_string(), limit));
    }
    let (open, close) = delimiters(kind);
    // The marker is written after a separator, so both are reserved.
    let fixed = open.len() + close.len() + TRUNCATION_SUFFIX.len() + 2;
    if depth >= MAX_CONTAINER_DEPTH || limit < fixed {
        return Some(summary(open, close, limit));
    }
    if let Some(length) = exact_len(value, kind)
        && length > MAX_CONTAINER_ITEMS.saturating_add(1)
    {
        // A container known to be oversized is summarised without copying a
        // prefix that only the summary would replace.
        return Some(summary(open, close, limit));
    }
    let Some(entries) = snapshot(value, kind) else {
        return Some(BoundedText::failure("<container unavailable>", limit));
    };
    Some(render_entries(&entries, kind, open, close, limit, depth))
}

/// Copy at most one entry past the rendering ceiling out of a container.
///
/// Taking the snapshot before any element is rendered means an element's
/// `__repr__` cannot mutate the container being iterated. One entry beyond the
/// ceiling is kept so that a container holding exactly the ceiling's worth is
/// not reported as truncated.
fn snapshot<'py>(value: &Bound<'py, PyAny>, kind: ContainerKind) -> Option<Vec<Piece<'py>>> {
    let mut entries = Vec::with_capacity(MAX_CONTAINER_ITEMS.min(4));
    if matches!(kind, ContainerKind::List) {
        // A list subclass can override `__iter__`, so the native iterator is
        // requested from the `list` type itself; that reads the real storage.
        let iterator = native_list_iterator(value)?;
        for item in iterator.take(MAX_CONTAINER_ITEMS.saturating_add(1)) {
            entries.push(Piece::Value(item.ok()?));
        }
        return Some(entries);
    }
    if matches!(kind, ContainerKind::Dict) {
        let dict = value.cast::<PyDict>().ok()?;
        for (key, entry) in dict.iter().take(MAX_CONTAINER_ITEMS.saturating_add(1)) {
            entries.push(Piece::Entry(key, entry));
        }
        return Some(entries);
    }
    let iterator = PyIterator::from_object(value).ok()?;
    for item in iterator.take(MAX_CONTAINER_ITEMS.saturating_add(1)) {
        entries.push(Piece::Value(item.ok()?));
    }
    Some(entries)
}

/// Read the exact length of a built-in container without iterating it.
fn exact_len(value: &Bound<'_, PyAny>, kind: ContainerKind) -> Option<usize> {
    match kind {
        ContainerKind::List => native_list_len(value),
        ContainerKind::Tuple => value.cast::<PyTuple>().ok().map(|tuple| tuple.len()),
        ContainerKind::Dict => value.cast::<PyDict>().ok().map(|dict| dict.len()),
        ContainerKind::Set => value.cast::<PySet>().ok().map(|set| set.len()),
        ContainerKind::FrozenSet => value.cast::<PyFrozenSet>().ok().map(|set| set.len()),
    }
}

/// Render a snapshotted container inside `open`/`close`.
fn render_entries(
    entries: &[Piece<'_>],
    kind: ContainerKind,
    open: &str,
    close: &str,
    limit: usize,
    depth: usize,
) -> BoundedText {
    // The marker is written after a separator, so both are reserved.
    let fixed = open.len() + close.len() + TRUNCATION_SUFFIX.len() + 2;
    let body_budget = limit - fixed;
    let mut body = String::new();
    let mut shortened = false;
    let mut byte_cut = false;
    let mut failed = false;
    let mut kept = 0usize;
    for (index, piece) in entries.iter().enumerate() {
        if index == MAX_CONTAINER_ITEMS {
            shortened = true;
            break;
        }
        let piece_limit = body_budget.saturating_sub(body.len()).max(1);
        let piece = match render_piece(piece, piece_limit, depth + 1) {
            Some(piece) => piece,
            None => {
                failed = true;
                break;
            }
        };
        shortened |= piece.truncated;
        failed |= piece.failed;
        if !fits(&mut body, &piece.text, body_budget) {
            shortened = true;
            byte_cut = true;
            break;
        }
        kept += 1;
    }
    let body = finish_body(body, kind, kept, entries.len(), shortened);
    let mut text = String::with_capacity(body.len() + fixed);
    text.push_str(open);
    text.push_str(&body);
    text.push_str(close);
    BoundedText {
        text,
        truncated: shortened,
        failed,
        // Only a cut by this call's byte allowance counts as clipped; item and
        // depth limits summarise without claiming the byte budget ran out.
        clipped: byte_cut,
    }
}

/// Add the marker and any singleton or empty-container form to a rendered body.
///
/// A singleton tuple keeps its trailing comma, and an empty `set` renders as
/// `set()` rather than as an empty dict. Both are decided here because they
/// depend on what the entry loop actually kept.
fn finish_body(
    mut body: String,
    kind: ContainerKind,
    kept: usize,
    offered: usize,
    shortened: bool,
) -> String {
    if shortened {
        if kept > 0 {
            body.push_str(", ");
        }
        body.push_str(TRUNCATION_SUFFIX);
    } else if is_single_tuple(kind, kept, offered) {
        // `(1)` is not a tuple; CPython prints `(1,)`.
        body.push(',');
    }
    body
}

/// Return `true` when a complete one-entry rendering is a singleton tuple.
fn is_single_tuple(kind: ContainerKind, kept: usize, offered: usize) -> bool {
    matches!(kind, ContainerKind::Tuple) && kept == 1 && offered == 1
}

/// Append `piece` to `body` when it fits `budget`, reporting whether it did.
///
/// The separator is written only together with a piece that fits, so a cut
/// never leaves a trailing comma that suggests another entry.
fn fits(body: &mut String, piece: &str, budget: usize) -> bool {
    let separator = usize::from(!body.is_empty()) * 2;
    if body.len() + separator + piece.len() > budget {
        return false;
    }
    if separator > 0 {
        body.push_str(", ");
    }
    body.push_str(piece);
    true
}

/// Render one entry of a container, pairing a dict key with its value.
fn render_piece(piece: &Piece<'_>, limit: usize, depth: usize) -> Option<BoundedText> {
    match piece {
        Piece::Value(value) => repr_bounded_at(value, limit, depth),
        Piece::Entry(key, value) => {
            let key = repr_bounded_at(key, limit, depth)?;
            let remaining = limit.saturating_sub(key.text.len() + 2).max(1);
            let value = repr_bounded_at(value, remaining, depth)?;
            let mut text = String::with_capacity(key.text.len() + value.text.len() + 2);
            text.push_str(&key.text);
            text.push_str(": ");
            text.push_str(&value.text);
            Some(BoundedText {
                text,
                truncated: key.truncated || value.truncated,
                failed: key.failed || value.failed,
                clipped: false,
            })
        }
    }
}

/// One entry to render: a value, or a dict key paired with its value.
enum Piece<'py> {
    /// A value from a sequence or set.
    Value(Bound<'py, PyAny>),
    /// A key and its value from a mapping.
    Entry(Bound<'py, PyAny>, Bound<'py, PyAny>),
}

/// The exact container type of `value`, if it is one of the recognised kinds.
#[derive(Clone, Copy)]
enum ContainerKind {
    /// `list`
    List,
    /// `tuple`
    Tuple,
    /// `dict`
    Dict,
    /// `set`
    Set,
    /// `frozenset`
    FrozenSet,
}

/// Classify `value` by its exact type.
fn container_kind(value: &Bound<'_, PyAny>) -> Option<ContainerKind> {
    if value.is_exact_instance_of::<PyList>() {
        return Some(ContainerKind::List);
    }
    if value.is_exact_instance_of::<PyTuple>() {
        return Some(ContainerKind::Tuple);
    }
    if value.is_exact_instance_of::<PyDict>() {
        return Some(ContainerKind::Dict);
    }
    if value.is_exact_instance_of::<PySet>() {
        return Some(ContainerKind::Set);
    }
    if inherits_list_repr(value) {
        return Some(ContainerKind::List);
    }
    if value.is_exact_instance_of::<PyFrozenSet>() {
        return Some(ContainerKind::FrozenSet);
    }
    None
}

/// Read a list subclass's length through the native `list` type.
fn native_list_len(value: &Bound<'_, PyAny>) -> Option<usize> {
    let len = value.py().get_type::<PyList>().getattr("__len__").ok()?;
    len.call1((value,)).ok()?.extract().ok()
}

/// Iterate a list subclass's storage without invoking its `__iter__`.
fn native_list_iterator<'py>(value: &Bound<'py, PyAny>) -> Option<Bound<'py, PyIterator>> {
    let iter = value.py().get_type::<PyList>().getattr("__iter__").ok()?;
    let iterator = iter.call1((value,)).ok()?;
    iterator.cast_into::<PyIterator>().ok()
}

/// Return `true` for a list subclass that inherits `list.__repr__`.
///
/// Such a subclass renders every entry through the inherited C
/// implementation, so it needs the same bounded element walk as an exact list.
/// A subclass with its own `__repr__` keeps the general path: that
/// representation is what the application chose to expose.
fn inherits_list_repr(value: &Bound<'_, PyAny>) -> bool {
    let py = value.py();
    let list_type = py.get_type::<PyList>();
    if !value.get_type().is_subclass(&list_type).unwrap_or(false) {
        return false;
    }
    let type_type = py.get_type::<pyo3::types::PyType>();
    let Ok(getattribute) = type_type.getattr("__getattribute__") else {
        return false;
    };
    let inherited = getattribute.call1((&list_type, "__repr__")).ok();
    let actual = getattribute.call1((value.get_type(), "__repr__")).ok();
    match (inherited, actual) {
        (Some(inherited), Some(actual)) => actual.is(&inherited),
        _ => false,
    }
}

/// The exact text of an empty container.
fn empty_form(kind: ContainerKind) -> &'static str {
    match kind {
        ContainerKind::List => "[]",
        ContainerKind::Tuple => "()",
        ContainerKind::Dict => "{}",
        ContainerKind::Set => "set()",
        ContainerKind::FrozenSet => "frozenset()",
    }
}

/// The opening and closing text of each container kind.
fn delimiters(kind: ContainerKind) -> (&'static str, &'static str) {
    match kind {
        ContainerKind::List => ("[", "]"),
        ContainerKind::Tuple => ("(", ")"),
        ContainerKind::Dict => ("{", "}"),
        ContainerKind::Set => ("{", "}"),
        ContainerKind::FrozenSet => ("frozenset({", "})"),
    }
}

/// A fixed summary used when a container is too deep or its ceiling too small.
fn summary(open: &str, close: &str, limit: usize) -> BoundedText {
    let bounded = clamp(format!("{open}{TRUNCATION_SUFFIX}{close}"), limit);
    BoundedText {
        text: bounded.text,
        truncated: true,
        failed: false,
        clipped: bounded.clipped,
    }
}

#[cfg(test)]
mod tests {
    //! Container rendering at the item and byte ceilings.

    use super::*;
    use pyo3::types::{PyFrozenSet, PySet};

    #[test]
    fn a_marker_separator_fits_inside_the_ceiling() {
        Python::attach(|py| {
            let list = PyList::new(py, ["12345", "6"]).expect("list should build");
            let rendered = render(list.as_any(), 12, 0).expect("a list should render");

            assert!(
                rendered.text.len() <= 12,
                "the marker separator must be part of the ceiling: {:?}",
                rendered.text
            );
            assert!(rendered.clipped, "the byte ceiling caused the cut");
        });
    }

    #[test]
    fn a_complete_container_is_not_reported_as_clipped() {
        Python::attach(|py| {
            let list = PyList::new(py, [1, 2]).expect("list should build");
            let rendered = render(list.as_any(), 32, 0).expect("a list should render");

            assert_eq!(rendered.text, "[1, 2]");
            assert!(!rendered.truncated);
            assert!(!rendered.clipped);
        });
    }

    #[test]
    fn singleton_tuples_and_empty_sets_keep_their_forms() {
        Python::attach(|py| {
            let tuple = PyTuple::new(py, [1]).expect("tuple should build");
            assert_eq!(render(tuple.as_any(), 16, 0).unwrap().text, "(1,)");

            let set = PySet::empty(py).expect("set should build");
            assert_eq!(render(set.as_any(), 16, 0).unwrap().text, "set()");

            let frozen = PyFrozenSet::empty(py).expect("frozenset should build");
            assert_eq!(render(frozen.as_any(), 16, 0).unwrap().text, "frozenset()");
        });
    }
}
