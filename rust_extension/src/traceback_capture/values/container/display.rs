//! Delimiters and fixed summaries for built-in container rendering.
//!
//! Container kinds, their Python delimiters, the empty forms and the fixed
//! summary all describe presentation rather than traversal. Keeping them in one
//! child module leaves the renderer under the repository's module-size ceiling.

use pyo3::prelude::*;
use pyo3::types::{PyFrozenSet, PySet};

use crate::traceback_capture::limits::TRUNCATION_SUFFIX;
use crate::traceback_capture::values::{BoundedText, clamp};

use super::empty_form;

/// The exact container type of `value`, if it is one of the recognised kinds.
#[derive(Clone, Copy)]
pub(super) enum ContainerKind {
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

/// Return the delimiters for a container, including a subclass's type name.
///
/// CPython renders a set or frozenset subclass as `Name({...})`. A subclass
/// that inherits the native representation keeps that wrapper; other kinds use
/// their base delimiters.
pub(super) fn display_delimiters(
    value: &Bound<'_, PyAny>,
    kind: ContainerKind,
    limit: usize,
) -> (String, String, String) {
    let (open, close) = delimiters(kind);
    let base = (
        open.to_string(),
        close.to_string(),
        empty_form(kind).to_string(),
    );
    if !matches!(kind, ContainerKind::Set | ContainerKind::FrozenSet) {
        return base;
    }
    let py = value.py();
    let native = match kind {
        ContainerKind::Set => py.get_type::<PySet>(),
        ContainerKind::FrozenSet => py.get_type::<PyFrozenSet>(),
        _ => return base,
    };
    if value.get_type().is(&native) {
        return base;
    }
    let Ok(name) = value.get_type().getattr("__name__") else {
        return base;
    };
    let Ok(name) = name.cast::<pyo3::types::PyString>() else {
        return base;
    };
    let name =
        crate::traceback_capture::values::bounded_string(name, limit.saturating_sub(3).max(1)).text;
    (format!("{name}({{"), "})".to_string(), format!("{name}()"))
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
pub(super) fn summary(open: &str, close: &str, limit: usize, byte_cut: bool) -> BoundedText {
    let bounded = clamp(format!("{open}{TRUNCATION_SUFFIX}{close}"), limit);
    BoundedText {
        text: bounded.text,
        truncated: true,
        failed: false,
        clipped: byte_cut || bounded.clipped,
    }
}
