//! Assembly of an exception's message from bounded argument representations.
//!
//! `super` reads and retains the arguments; this module turns them into the
//! text `BaseException.__str__` would have produced, without calling user code
//! again. Keeping the message shapes separate leaves both files under the
//! repository's module-size ceiling and keeps the argument-reading rules in one
//! place.

use pyo3::exceptions::PySyntaxError;
use pyo3::prelude::*;
use pyo3::types::{PyString, PyType};

use crate::traceback_capture::limits::{STR_FAILURE_SENTINEL, TRUNCATION_SUFFIX};
use crate::traceback_capture::values::{self, BoundedText};

use super::RenderedArgs;

/// Build the exception's message, clamped to `limit` bytes.
///
/// `str(exception)` is called only where it is cheap or cannot be replaced. An
/// exception with one argument has nothing to re-render beyond the
/// representation already taken, and one whose type overrides `__str__` has a
/// message the argument tuple cannot reproduce. Only the default multi-argument
/// case — where `str` would mean one `__repr__` per argument — is assembled
/// from the representations instead.
pub(in crate::traceback_capture) fn build_message(
    exception: &Bound<'_, PyAny>,
    args: &RenderedArgs,
    limit: usize,
) -> BoundedText {
    if let Some(message) = syntax_error_message(exception, limit) {
        return message;
    }
    if let Some(message) = key_error_message(exception, args, limit) {
        return message;
    }
    if uses_default_str(exception).unwrap_or(false) {
        if args.total > 1 {
            return multi_argument_message(args, limit);
        }
        if let Some(argument) = args.first_arg.as_ref() {
            return values::stringify_argument(argument.bind(exception.py()), limit);
        }
    }
    values::stringify(exception, limit, STR_FAILURE_SENTINEL)
}

/// Assemble the multi-argument message and mark it when arguments are missing.
///
/// `joined_args` renders a `...` marker for arguments the count ceiling dropped
/// or a failed representation left as a gap. That marker is part of the message
/// even when the assembled text fits its byte ceiling, so the message is
/// reported as shortened rather than complete.
fn multi_argument_message(args: &RenderedArgs, limit: usize) -> BoundedText {
    let mut message = values::clamp(joined_args(args), limit);
    if args.has_omissions() {
        message.truncated = true;
    }
    message
}

/// Preserve the message field exposed by `TracebackException` for syntax errors.
fn syntax_error_message(exception: &Bound<'_, PyAny>, limit: usize) -> Option<BoundedText> {
    let py = exception.py();
    // The real type decides, not `__class__`: a subclass may expose a property
    // claiming to be a `SyntaxError` while its own `__str__` says otherwise.
    let syntax_error = py.get_type::<PySyntaxError>();
    if !exception
        .get_type()
        .is_subclass(&syntax_error)
        .unwrap_or(false)
    {
        return None;
    }
    let message = exception.getattr("msg").ok()?;
    let message = message.cast::<PyString>().ok()?;
    Some(values::bounded_string(message, limit))
}

/// Report whether `exception` inherits `BaseException.__str__` unchanged.
///
/// Returns `None` when the comparison cannot be made, which leaves the caller
/// on the `str` path — the safe direction, because the joined form is only
/// correct for types whose `__str__` really is the default.
fn uses_default_str(exception: &Bound<'_, PyAny>) -> Option<bool> {
    let py = exception.py();
    let base = py
        .import("builtins")
        .and_then(|builtins| builtins.getattr("BaseException"))
        .ok()?;
    let inherited = class_attribute(&base, "__str__")?;
    let actual = class_attribute(exception.get_type().as_any(), "__str__")?;
    // Identity, not equality: a subclass that copied the body but redefined the
    // slot still counts as overriding, which is the conservative reading.
    Some(actual.is(&inherited))
}

/// Read a class attribute through the ordinary type lookup.
///
/// `cls.__str__` goes through the metaclass's `__getattribute__`, which
/// application code can override, so the value compared here could be produced
/// by user code rather than read from the class's method-resolution order.
/// Calling `type.__getattribute__` directly performs the MRO lookup the
/// comparison needs.
fn class_attribute<'py>(class: &Bound<'py, PyAny>, name: &str) -> Option<Bound<'py, PyAny>> {
    let py = class.py();
    let type_type = py.get_type::<PyType>();
    let getattribute = type_type.getattr("__getattribute__").ok()?;
    getattribute.call1((class, name)).ok()
}

/// Reproduce `KeyError.__str__` from the argument representations already taken.
///
/// CPython renders a single-argument `KeyError` as `repr(args[0])` and the
/// multi-argument form as the default `repr(args)`. Both are exactly what
/// [`render_args`] and [`joined_args`] already produce, so using them avoids a
/// second trip through user code — and avoids the unbounded intermediate string
/// that `str(a_large_key)` would build before the ceiling could apply.
///
/// A subclass that overrides `__str__` is left alone: its own rendering is the
/// correct message, so the identity check fails and the caller falls back to
/// `str`.
fn key_error_message(
    exception: &Bound<'_, PyAny>,
    args: &RenderedArgs,
    limit: usize,
) -> Option<BoundedText> {
    let py = exception.py();
    let key_error = py.import("builtins").ok()?.getattr("KeyError").ok()?;
    if !exception
        .get_type()
        .is_subclass(&key_error)
        .unwrap_or(false)
    {
        return None;
    }
    let inherited = class_attribute(&key_error, "__str__")?;
    let actual = class_attribute(exception.get_type().as_any(), "__str__")?;
    if !actual.is(&inherited) {
        return None;
    }
    if args.total > 1 {
        return Some(multi_argument_message(args, limit));
    }
    match args.texts.first() {
        Some(text) => {
            // The key's representation was already bounded by `max_arg_bytes`.
            // A cut there is a cut in the message too, so carry the flag rather
            // than reporting an untruncated message built from shortened text.
            let mut bounded = values::clamp(text.clone(), limit);
            bounded.truncated |= args.truncated;
            Some(bounded)
        }
        // The only key could not be represented. Returning `None` would send
        // the caller to `str(exception)`, which calls the same failing
        // `__repr__` a second time, so report the contained failure instead.
        None if args.failed > 0 => Some(BoundedText::failure("<repr conversion failed>", limit)),
        None => None,
    }
}

/// Assemble CPython's multi-argument message from bounded representations.
///
/// The result mirrors the shape `BaseException.__str__` produces — the `args`
/// tuple's `repr` — so a message stays recognisable, but it is built from the
/// representations already retained rather than by re-running user code. An
/// argument the count or size ceiling dropped appears as `...`, which keeps the
/// message honest about being a prefix.
pub(super) fn joined_args(args: &RenderedArgs) -> String {
    // A failed representation in the middle is a gap. Later texts are retained
    // for `args_repr`, but displaying them here would move them into the gap's
    // position, so the message stops where the gap starts.
    let keep = args
        .first_failed
        .map_or(args.texts.len(), |gap| gap.min(args.texts.len()));
    let mut rendered = String::from("(");
    for (index, text) in args.texts.iter().take(keep).enumerate() {
        if index > 0 {
            rendered.push_str(", ");
        }
        rendered.push_str(text);
    }
    if args.first_failed.is_some() || args.is_incomplete() {
        if keep > 0 {
            rendered.push_str(", ");
        }
        rendered.push_str(TRUNCATION_SUFFIX);
    }
    rendered.push(')');
    rendered
}
