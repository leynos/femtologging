//! Bounded rendering of an exception's message and the arguments behind it.
//!
//! CPython's `BaseException.__str__` has two shapes: with no argument or one it
//! returns that argument's `str`, and with more it returns the `repr` of the
//! whole `args` tuple, which runs `__repr__` on every argument. Calling `str`
//! on a multi-argument exception therefore invokes user code once per argument
//! for a result the walker discards, and a single raising `__repr__` makes the
//! entire message unavailable while the arguments themselves are still
//! perfectly renderable.
//!
//! The multi-argument form is therefore assembled here from representations
//! taken once, so each argument's `__repr__` runs exactly once per capture and
//! the message inherits the argument ceilings. The single-argument form is
//! still delegated to Python, because `str(argument)` reaches an argument's
//! `__str__` — a hook no representation here would otherwise call.
//!
//! `KeyError` is the exception: its own `__str__` is `repr(args[0])`, which is
//! the representation already taken, so the bounded copy is used directly
//! rather than asking Python to build the whole thing again.
//!
//! The arguments are read through `BaseException.args` rather than the
//! instance, so a subclass cannot substitute a property that lies about the
//! tuple or costs an unbounded read. `__str__` identity is likewise read with
//! `type.__getattribute__`, keeping an interposing metaclass out of the check.
//!
//! # Overridden `__str__` wins
//!
//! The `repr(args)` shape is what the *default* `__str__` produces, so it is
//! only substituted for types that actually use it. Many built-ins with several
//! arguments override `__str__` to something far more useful — `OSError(2, 'x')`
//! renders as `[Errno 2] x`, `ExceptionGroup` as `outer (2 sub-exceptions)` —
//! and replacing those with the raw argument tuple would be a regression. Any
//! type with its own `__str__` is delegated to it, which costs one user call
//! rather than one per argument.

use pyo3::prelude::*;
use pyo3::types::{PyString, PyTuple, PyType};

use super::attrs::as_u32;
use super::limits::{CaptureBudget, CaptureLimits, STR_FAILURE_SENTINEL, TRUNCATION_SUFFIX};
use super::values::{self, BoundedText};
use crate::exception_schema::PayloadTruncation;

/// Argument representations read before the aggregate budget decides on them.
///
/// Rendering is separate from retention because the message is built from these
/// representations, and the message is charged before the arguments are. Holding
/// the rendered text briefly is what lets the budget keep its documented spend
/// order — type, message, frames, arguments, notes — while each argument's
/// `__repr__` still runs only once.
///
/// The render's own outcomes are carried here rather than written straight to a
/// marker, because whether they are worth reporting depends on the node: only
/// the exception capture was asked about retains a `args_repr` field, so a
/// truncation flag on a descendant would describe a field its payload does not
/// carry.
pub(super) struct RenderedArgs {
    /// Representations read so far, one per argument rendered.
    texts: Vec<String>,
    /// How many arguments the exception actually carries.
    total: usize,
    /// How many argument representations raised and were skipped.
    failed: u32,
    /// Whether any retained representation was shortened.
    pub(super) truncated: bool,
    /// First original value, used for bounded `str(value)` parity.
    first_arg: Option<Py<PyAny>>,
}

impl RenderedArgs {
    /// The representation recorded for an exception with no usable `args`.
    fn none() -> Self {
        Self {
            texts: Vec::new(),
            total: 0,
            failed: 0,
            truncated: false,
            first_arg: None,
        }
    }

    /// Record an exception whose `args` attribute could not be read.
    fn failed() -> Self {
        Self {
            failed: 1,
            ..Self::none()
        }
    }

    /// Return `true` when some argument is missing from [`Self::texts`].
    fn is_incomplete(&self) -> bool {
        self.texts.len() < self.total
    }

    /// Report argument conversions that failed while building a node message.
    ///
    /// Descendant nodes also use their representations to build a message, even
    /// though their `args_repr` field stays empty.
    pub(super) fn report_failures(&self, marker: &mut PayloadTruncation) {
        marker.failed_elements = marker.failed_elements.saturating_add(self.failed);
    }
}

/// Read up to `max_args` argument representations, without charging them.
///
/// When `should_render` is false, count the arguments but do not invoke their
/// `__repr__` methods. This is used after the aggregate budget is exhausted.
///
/// A failing `repr` skips the element rather than the call, which is the
/// library's existing partial-extraction rule, and is counted so a consumer can
/// tell an unrenderable argument from one the ceilings dropped.
pub(super) fn render_args(
    exception: &Bound<'_, PyAny>,
    limits: &CaptureLimits,
    should_render: bool,
) -> RenderedArgs {
    let args = match exception_args(exception) {
        Ok(args) if !args.is_none() => args,
        Ok(_) => return RenderedArgs::none(),
        Err(_) => return RenderedArgs::failed(),
    };
    // `cast` borrows its argument, so the binding has to outlive the cast rather
    // than living inside a closure's parameter.
    let Ok(args) = args.cast::<PyTuple>() else {
        return RenderedArgs::none();
    };
    let total = args.len();
    if !should_render {
        return RenderedArgs {
            total,
            ..RenderedArgs::none()
        };
    }
    let mut rendered = RenderedArgs {
        texts: Vec::with_capacity(total.min(limits.max_args)),
        total,
        failed: 0,
        truncated: false,
        first_arg: None,
    };
    for (index, arg) in args.iter().take(limits.max_args).enumerate() {
        if index == 0 {
            rendered.first_arg = Some(arg.clone().unbind());
        }
        let Some(text) = values::repr_bounded(&arg, limits.max_arg_bytes) else {
            rendered.failed += 1;
            continue;
        };
        if text.failed {
            rendered.failed += 1;
            continue;
        }
        rendered.truncated |= text.truncated;
        rendered.texts.push(text.text);
    }
    rendered
}

/// Read the exception's real `args` tuple through `BaseException`'s descriptor.
///
/// Reading `exception.args` consults the instance and then its class, so a
/// subclass can expose a property that lies about the arguments or builds an
/// arbitrarily large value. The base descriptor reads the C-level field that
/// `BaseException.__str__` itself uses, and calling it directly keeps a
/// user-defined `args` property out of the capture.
fn exception_args<'py>(exception: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let py = exception.py();
    let base = py.import("builtins")?.getattr("BaseException")?;
    let descriptor = base.getattr("args")?;
    descriptor.call_method1("__get__", (exception, exception.get_type()))
}

/// Charge the rendered arguments against `budget`, keeping what still fits.
///
/// Arguments that do not fit are reported as omitted, so the marker accounts for
/// them the same way the count ceiling does. An argument whose `__repr__` raised
/// is reported through `failed_elements` instead, so it is not counted twice.
pub(super) fn retain_args(
    rendered: RenderedArgs,
    budget: &mut CaptureBudget,
    marker: &mut PayloadTruncation,
) -> Vec<String> {
    let mut retained = Vec::with_capacity(rendered.texts.len());
    for text in rendered.texts {
        if !budget.charge_bytes(text.len()) {
            break;
        }
        retained.push(text);
    }
    let unavailable = as_u32(rendered.total.saturating_sub(retained.len()));
    let omitted = unavailable.saturating_sub(rendered.failed);
    marker.args_omitted = marker.args_omitted.saturating_add(omitted);
    retained
}

/// Build the exception's message, clamped to `limit` bytes.
///
/// `str(exception)` is called only where it is cheap or cannot be replaced. An
/// exception with one argument has nothing to re-render beyond the
/// representation already taken, and one whose type overrides `__str__` has a
/// message the argument tuple cannot reproduce. Only the default multi-argument
/// case — where `str` would mean one `__repr__` per argument — is assembled
/// from the representations instead.
pub(super) fn build_message(
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
            return values::clamp(joined_args(args), limit);
        }
        if let Some(argument) = args.first_arg.as_ref() {
            return values::stringify_argument(argument.bind(exception.py()), limit);
        }
    }
    values::stringify(exception, limit, STR_FAILURE_SENTINEL)
}

/// Preserve the message field exposed by `TracebackException` for syntax errors.
fn syntax_error_message(exception: &Bound<'_, PyAny>, limit: usize) -> Option<BoundedText> {
    let py = exception.py();
    let syntax_error = py
        .import("builtins")
        .and_then(|builtins| builtins.getattr("SyntaxError"))
        .ok()?;
    if !exception.is_instance(&syntax_error).ok()? {
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
        return Some(values::clamp(joined_args(args), limit));
    }
    args.texts
        .first()
        .map(|text| values::clamp(text.clone(), limit))
}

/// Assemble CPython's multi-argument message from bounded representations.
///
/// The result mirrors the shape `BaseException.__str__` produces — the `args`
/// tuple's `repr` — so a message stays recognisable, but it is built from the
/// representations already retained rather than by re-running user code. An
/// argument the count or size ceiling dropped appears as `...`, which keeps the
/// message honest about being a prefix.
fn joined_args(args: &RenderedArgs) -> String {
    let mut rendered = String::from("(");
    for (index, text) in args.texts.iter().enumerate() {
        if index > 0 {
            rendered.push_str(", ");
        }
        rendered.push_str(text);
    }
    if args.is_incomplete() {
        if !args.texts.is_empty() {
            rendered.push_str(", ");
        }
        rendered.push_str(TRUNCATION_SUFFIX);
    }
    rendered.push(')');
    rendered
}

#[cfg(test)]
#[path = "message_tests.rs"]
mod tests;
