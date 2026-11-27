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
use pyo3::types::{PyDict, PyFrozenSet, PyIterator, PyList, PySet, PyTuple, PyType};

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
    if depth >= MAX_CONTAINER_DEPTH {
        // A depth stop is not a byte cut: the summary itself may fit.
        return Some(summary(open, close, limit, false));
    }
    let fixed = open.len() + close.len();
    if limit <= fixed {
        return Some(summary(open, close, limit, true));
    }
    if let Some(length) = exact_len(value, kind)
        && length > MAX_CONTAINER_ITEMS.saturating_add(1)
    {
        // The item ceiling, not the byte ceiling, stops this container.
        return Some(summary(open, close, limit, false));
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
    if matches!(kind, ContainerKind::Dict) {
        // A subclass can override `items`, so the view is requested from the
        // native `dict` type. Pairing happens before any element is rendered.
        let dict_type = value.py().get_type::<PyDict>();
        let items = dict_type.getattr("items").ok()?.call1((value,)).ok()?;
        let iterator = PyIterator::from_object(&items).ok()?;
        for item in iterator.take(MAX_CONTAINER_ITEMS.saturating_add(1)) {
            let pair = item.ok()?.cast_into::<PyTuple>().ok()?;
            entries.push(Piece::Entry(pair.get_item(0).ok()?, pair.get_item(1).ok()?));
        }
        return Some(entries);
    }
    // A subclass can override `__iter__`, so the native iterator is requested
    // from the built-in type itself; that reads the real storage.
    let native = match kind {
        ContainerKind::List => value.py().get_type::<PyList>(),
        ContainerKind::Tuple => value.py().get_type::<PyTuple>(),
        ContainerKind::Set => value.py().get_type::<PySet>(),
        ContainerKind::FrozenSet => value.py().get_type::<PyFrozenSet>(),
        ContainerKind::Dict => unreachable!("dict handled above"),
    };
    let iterator_object = native.getattr("__iter__").ok()?.call1((value,)).ok()?;
    let iterator = PyIterator::from_object(&iterator_object).ok()?;
    for item in iterator.take(MAX_CONTAINER_ITEMS.saturating_add(1)) {
        entries.push(Piece::Value(item.ok()?));
    }
    Some(entries)
}

/// Read the length of a built-in container without invoking a subclass hook.
fn exact_len(value: &Bound<'_, PyAny>, kind: ContainerKind) -> Option<usize> {
    let py = value.py();
    let native = match kind {
        ContainerKind::List => py.get_type::<PyList>(),
        ContainerKind::Tuple => py.get_type::<PyTuple>(),
        ContainerKind::Dict => py.get_type::<PyDict>(),
        ContainerKind::Set => py.get_type::<PySet>(),
        ContainerKind::FrozenSet => py.get_type::<PyFrozenSet>(),
    };
    native
        .getattr("__len__")
        .ok()?
        .call1((value,))
        .ok()?
        .extract()
        .ok()
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
    let body_budget = limit - open.len() - close.len();
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
    let (body, reclaimed) = finish_body(body, kind, kept, entries.len(), shortened, body_budget);
    byte_cut |= reclaimed;
    let mut text = String::with_capacity(body.len() + open.len() + close.len());
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

/// Add the marker, reclaiming complete entries when the byte ceiling is full.
///
/// Entries are rendered into the space the delimiters leave. When the result
/// turns out to be shortened, the marker and its separator must also fit, so
/// entries are removed from the end at their separators until they do. Returns
/// the body and whether the byte ceiling forced an entry out.
fn finish_body(
    mut body: String,
    kind: ContainerKind,
    mut kept: usize,
    offered: usize,
    shortened: bool,
    budget: usize,
) -> (String, bool) {
    if !shortened {
        if is_single_tuple(kind, kept, offered) {
            // `(1)` is not a tuple; CPython prints `(1,)`.
            body.push(',');
        }
        return (body, false);
    }
    let mut reclaimed = false;
    loop {
        let separator = usize::from(kept > 0) * 2;
        if body.len() + separator + TRUNCATION_SUFFIX.len() <= budget {
            break;
        }
        if kept == 0 {
            // The marker alone does not fit; keep the part of it the ceiling
            // allows rather than overflowing.
            let room = budget.min(TRUNCATION_SUFFIX.len());
            body.clear();
            body.push_str(&TRUNCATION_SUFFIX[..room]);
            return (body, true);
        }
        reclaimed = true;
        kept -= 1;
        match body.rfind(", ") {
            Some(index) => body.truncate(index),
            None => body.clear(),
        }
    }
    if kept > 0 {
        body.push_str(", ");
    }
    body.push_str(TRUNCATION_SUFFIX);
    (body, reclaimed)
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

/// Classify `value` by its built-in container type.
///
/// An exact instance always qualifies. A subclass qualifies only when it
/// inherits the native `__repr__` unchanged; a genuine override keeps the
/// general path, because that representation is what the application chose to
/// expose.
fn container_kind(value: &Bound<'_, PyAny>) -> Option<ContainerKind> {
    let py = value.py();
    let candidates = [
        (py.get_type::<PyList>(), ContainerKind::List),
        (py.get_type::<PyTuple>(), ContainerKind::Tuple),
        (py.get_type::<PyDict>(), ContainerKind::Dict),
        (py.get_type::<PySet>(), ContainerKind::Set),
        (py.get_type::<PyFrozenSet>(), ContainerKind::FrozenSet),
    ];
    for (native, kind) in candidates {
        if value.get_type().is(&native) {
            return Some(kind);
        }
        if value.get_type().is_subclass(&native).unwrap_or(false)
            && inherits_native(value, &native, "__repr__")
        {
            return Some(kind);
        }
    }
    None
}

/// Return `true` when `value` inherits `method` unchanged from `native`.
///
/// The comparison uses `type.__getattribute__` so a metaclass cannot fake the
/// lookup. A genuine override keeps the general path: that representation is
/// what the application chose to expose.
fn inherits_native(value: &Bound<'_, PyAny>, native: &Bound<'_, PyType>, method: &str) -> bool {
    let py = value.py();
    let type_type = py.get_type::<PyType>();
    let Ok(getattribute) = type_type.getattr("__getattribute__") else {
        return false;
    };
    let inherited = getattribute.call1((native, method)).ok();
    let actual = getattribute.call1((value.get_type(), method)).ok();
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
fn summary(open: &str, close: &str, limit: usize, byte_cut: bool) -> BoundedText {
    let bounded = clamp(format!("{open}{TRUNCATION_SUFFIX}{close}"), limit);
    BoundedText {
        text: bounded.text,
        truncated: true,
        failed: false,
        clipped: byte_cut || bounded.clipped,
    }
}
