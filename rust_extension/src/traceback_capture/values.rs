//! Bounded stringification of values that user code controls.
//!
//! Every conversion here runs potentially user-supplied Python code
//! (`__str__`, `__repr__`) and every one of them is contained: a raising hook
//! is recorded and captured as a placeholder rather than propagated into the
//! logging call.
//!
//! Containing a *failure* is not enough on its own, because a conversion that
//! succeeds can still build an arbitrarily large string. The built-in string and
//! byte types are therefore recognised before the general path: their exact
//! content is sliced down to the ceiling first, and the slice — not the original
//! value — is what gets escaped or copied. Rendering a one-megabyte `str`
//! argument to `repr` therefore costs the ceiling, not the argument.
//!
//! A sliced `str` is handed back to Python's `repr`, so quote selection and
//! `\xNN` escapes match what a caller would see from an untruncated payload. A
//! `bytes` or `bytearray` is escaped here byte by byte for the same reason: the
//! `repr` of a large blob would be paid for in full before the ceiling could
//! apply, and a lossy decode would not reproduce CPython's escapes.

use pyo3::prelude::*;
use pyo3::types::{PySlice, PyString, PyStringMethods};

use super::limits::{TRUNCATION_SUFFIX_LEN, truncate_to_limit};

mod container;
mod literal;
use literal::{bounded_rendered, force_truncate, render_byte_like};

/// A value that was converted to text, together with how it was obtained.
pub(super) struct BoundedText {
    /// Retained text, already inside the caller's byte ceiling.
    pub text: String,
    /// `true` when the source value was longer than the ceiling allowed.
    pub truncated: bool,
    /// `true` when converting the value raised and [`BoundedText::text`] holds
    /// a placeholder instead of the real value.
    pub failed: bool,
    /// `true` when this call's own byte ceiling cut the text.
    ///
    /// Distinct from [`Self::truncated`], which can also be inherited from an
    /// input that was already shortened: a message built from a shortened
    /// `KeyError` key is marked truncated, but its text was not cut by the
    /// message ceiling.
    pub clipped: bool,
}

impl BoundedText {
    /// Record a conversion failure with `placeholder` inside the byte ceiling.
    pub(super) fn failure(placeholder: &str, limit: usize) -> Self {
        let truncated = placeholder.len() > limit;
        let text = truncate_to_limit(placeholder, limit).unwrap_or_else(|| placeholder.to_string());
        Self {
            text,
            truncated,
            failed: true,
            clipped: truncated,
        }
    }

    /// Record a successful conversion of `text`.
    fn ok(text: String, truncated: bool) -> Self {
        Self {
            text,
            truncated,
            clipped: truncated,
            failed: false,
        }
    }
}

/// Apply `limit` to `text`, reporting whether the marker was added.
pub(super) fn clamp(text: String, limit: usize) -> BoundedText {
    match truncate_to_limit(&text, limit) {
        Some(shortened) => BoundedText::ok(shortened, true),
        None => BoundedText::ok(text, false),
    }
}

/// Convert `value` to `str`, containing any failure.
///
/// `placeholder` is retained when the conversion raises. This is the outermost
/// containment point for exception messages.
pub(super) fn stringify(value: &Bound<'_, PyAny>, limit: usize, placeholder: &str) -> BoundedText {
    let Ok(rendered) = value.str() else {
        return BoundedText::failure(placeholder, limit);
    };
    bounded_string(&rendered, limit)
}

/// Convert `value` to `repr`, containing any failure.
///
/// Returns `None` when the value has no usable representation; callers treat
/// that as an element to skip and count.
pub(super) fn repr_bounded(value: &Bound<'_, PyAny>, limit: usize) -> Option<BoundedText> {
    repr_bounded_at(value, limit, 0)
}

/// Render `value` at container nesting `depth`.
///
/// Containers are rendered element by element so a huge or group-heavy element
/// cannot build the whole representation first. The depth counter bounds that
/// recursion; [`container::render`] summarises anything nested too deeply.
pub(super) fn repr_bounded_at(
    value: &Bound<'_, PyAny>,
    limit: usize,
    depth: usize,
) -> Option<BoundedText> {
    if let Some(rendered) = container::render(value, limit, depth) {
        return Some(rendered);
    }
    // CPython's exception-group repr reproduces its whole subtree, and a group
    // that shares child groups multiplies that work at every level. The group
    // walker captures the members separately, so the textual representation is
    // replaced by a fixed placeholder instead of being built here.
    if super::types::is_exception_group(value.py(), value) {
        return Some(clamp("<exception group>".to_string(), limit));
    }
    if let Some(rendered) = render_builtin(value, limit) {
        return Some(rendered);
    }
    let binding = value.repr().ok()?;
    Some(bounded_string(&binding, limit))
}

/// Convert a built-in string to bounded UTF-8 without copying its full value.
///
/// Python strings can contain lone surrogates, which `to_str()` cannot borrow
/// as UTF-8. For those values the bounded prefix is sliced in Python first,
/// then converted lossily; the conversion therefore allocates in proportion to
/// `limit`, even when the original string is very large.
pub(super) fn bounded_string(value: &Bound<'_, PyString>, limit: usize) -> BoundedText {
    let Ok(code_points) = value.code_point_len() else {
        return BoundedText::failure("<string conversion failed>", limit);
    };
    if code_points <= limit {
        return clamp(value.to_string_lossy().into_owned(), limit);
    }

    let Some(prefix) = native_prefix(value, limit) else {
        return BoundedText::failure("<string conversion failed>", limit);
    };
    let text = prefix.to_string_lossy().into_owned();
    BoundedText::ok(force_truncate(&text, limit), true)
}

/// Return `value[:stop]` evaluated by `str` itself.
///
/// Slicing through the instance would dispatch to a subclass's `__getitem__`,
/// which runs user code and may return anything at all. The unbound
/// `str.__getitem__` reads the string's own storage, so the bounded prefix is
/// always a real string and never a user-defined result.
fn native_prefix<'py>(value: &Bound<'py, PyString>, stop: usize) -> Option<Bound<'py, PyString>> {
    let py = value.py();
    let getitem = py.get_type::<PyString>().getattr("__getitem__").ok()?;
    let stop = isize::try_from(stop).unwrap_or(isize::MAX);
    let slice = PySlice::new(py, 0, stop, 1);
    getitem.call1((value, slice)).ok()?.cast_into().ok()
}

/// Convert the single argument used by `BaseException.__str__` safely.
///
/// `str(bytes)` and `str(bytearray)` construct representations of the complete
/// value in CPython. Render their bounded prefix directly so a large binary
/// argument cannot allocate an oversized intermediate message.
pub(super) fn stringify_argument(value: &Bound<'_, PyAny>, limit: usize) -> BoundedText {
    // Only the exact type takes the copy path: a subclass may override
    // `__str__`, and `str(argument)` is the semantic this function reproduces.
    if value.is_exact_instance_of::<PyString>()
        && let Ok(text) = value.cast::<PyString>()
    {
        return bounded_string(text, limit);
    }
    // `str` of an exact built-in container is its `repr`, so the same bounded
    // element rendering applies without going through Python.
    if let Some(rendered) = container::render(value, limit, 0) {
        return rendered;
    }
    if let Some((rendered, truncated)) = render_byte_like(value) {
        return bounded_rendered(rendered, truncated, limit);
    }
    stringify(value, limit, "<exception str() failed>")
}

/// Render a built-in string or byte-like value from a bounded slice.
///
/// Returns `None` for any other type, leaving the caller to take the general
/// `repr` path.
fn render_builtin(value: &Bound<'_, PyAny>, limit: usize) -> Option<BoundedText> {
    if let Some((rendered, truncated)) = render_byte_like(value) {
        return Some(bounded_rendered(rendered, truncated, limit));
    }
    render_long_string(value, limit)
}

/// Render an oversized `str` from a bounded prefix.
///
/// Returns `None` when the value is not an exact `str` or is not exposed as
/// UTF-8 — a string holding lone surrogates — leaving the caller to take the
/// general path. A `str` subclass is excluded because it may override
/// `__repr__`, which this bounded prefix would bypass.
///
/// A source that itself fits is still rendered through `repr`, because escapes
/// can expand it past the ceiling; a longer source is sliced first. Both cases
/// end in the literal-aware clipping that keeps the closing quote.
fn render_long_string(value: &Bound<'_, PyAny>, limit: usize) -> Option<BoundedText> {
    if !value.is_exact_instance_of::<PyString>() {
        return None;
    }
    let text = value.cast::<PyString>().ok()?;
    let Ok(code_points) = text.code_point_len() else {
        // A non-canonical string cannot be sliced here; the general `repr`
        // path still produces a usable representation.
        return None;
    };
    if code_points <= limit {
        // The input fits, but its escaped representation may not: a short run
        // of NULs expands fourfold. `repr` is safe to call on the whole value
        // because the value itself is already inside the ceiling.
        let Ok(rendered) = text.repr() else {
            return Some(BoundedText::failure("<repr conversion failed>", limit));
        };
        let rendered = rendered.to_string_lossy().into_owned();
        return Some(bounded_rendered(rendered, false, limit));
    }
    let reserve = TRUNCATION_SUFFIX_LEN.saturating_add(2);
    let stop = limit.saturating_sub(reserve);
    let Some(prefix) = native_prefix(text, stop) else {
        return Some(BoundedText::failure("<repr conversion failed>", limit));
    };
    let Ok(rendered) = prefix.repr() else {
        return Some(BoundedText::failure("<repr conversion failed>", limit));
    };
    let rendered = rendered.to_string_lossy().into_owned();
    Some(bounded_rendered(rendered, true, limit))
}
