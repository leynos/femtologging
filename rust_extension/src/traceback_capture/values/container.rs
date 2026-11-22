//! Bounded rendering of exact built-in containers.
//!
//! `repr` of a list, tuple, dict, set or frozenset renders every element and
//! builds the whole text before any byte ceiling can apply. One argument holding
//! a million entries, or holding an exception group whose own `repr` reproduces
//! a shared subtree, would then cost far more than the capture budget on the
//! logging thread.
//!
//! Each container is therefore rendered here, element by element, stopping at
//! explicit item, depth and byte limits. Small containers keep the ordinary
//! representation because the element rendering is the same text; a container
//! that would exceed a limit is summarised with the truncation marker instead.
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
pub(super) fn render(value: &Bound<'_, PyAny>, limit: usize, depth: usize) -> Option<BoundedText> {
    let kind = container_kind(value)?;
    let (open, close) = delimiters(kind);
    let fixed = open.len() + close.len() + TRUNCATION_SUFFIX.len();
    if depth >= MAX_CONTAINER_DEPTH || limit < fixed {
        return Some(summary(open, close, limit));
    }
    match kind {
        ContainerKind::Dict => {
            let dict = value.cast::<PyDict>().ok()?;
            render_entries(
                dict.iter().map(|(key, entry)| Ok(Piece::Entry(key, entry))),
                open,
                close,
                limit,
                depth,
            )
        }
        _ => {
            let iterator = PyIterator::from_object(value).ok()?;
            render_entries(
                iterator.map(|item| item.map(Piece::Value)),
                open,
                close,
                limit,
                depth,
            )
        }
    }
}

/// Render entries from `iterator` inside `open`/`close`, stopping at the ceilings.
fn render_entries<'py>(
    iterator: impl Iterator<Item = PyResult<Piece<'py>>>,
    open: &str,
    close: &str,
    limit: usize,
    depth: usize,
) -> Option<BoundedText> {
    let fixed = open.len() + close.len() + TRUNCATION_SUFFIX.len();
    let body_budget = limit - fixed;
    let mut body = String::new();
    let mut truncated = false;
    for (rendered, piece) in iterator.enumerate() {
        if rendered == MAX_CONTAINER_ITEMS {
            truncated = true;
            break;
        }
        let piece = piece.ok()?;
        let piece_limit = body_budget.saturating_sub(body.len()).max(1);
        let piece = render_piece(&piece, piece_limit, depth + 1)?;
        if !fits(&mut body, &piece.text, body_budget) {
            truncated = true;
            break;
        }
    }
    let mut text = String::with_capacity(body.len() + fixed);
    text.push_str(open);
    text.push_str(&body);
    if truncated {
        text.push_str(TRUNCATION_SUFFIX);
    }
    text.push_str(close);
    Some(BoundedText::ok(text, truncated))
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
            Some(BoundedText::ok(text, key.truncated || value.truncated))
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
    if value.is_exact_instance_of::<PyFrozenSet>() {
        return Some(ContainerKind::FrozenSet);
    }
    None
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
    BoundedText::ok(bounded.text, true)
}
