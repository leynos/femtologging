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
use pyo3::types::{PyIterator, PyTuple};

use super::attrs::as_u32;
use super::limits::{CaptureBudget, CaptureLimits, TRUNCATION_SUFFIX};
use super::types;
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
    /// Whether each representation in [`Self::texts`] was shortened.
    shortened: Vec<bool>,
    /// How many arguments the exception actually carries.
    total: usize,
    /// How many argument representations raised and were skipped.
    failed: u32,
    /// Position of the first argument whose representation failed.
    ///
    /// A failure in the middle of the tuple is a gap, not a suffix: rendering
    /// the surviving texts in order would silently move later arguments into
    /// earlier positions. The message stops at this position instead.
    first_failed: Option<usize>,
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
            shortened: Vec::new(),
            total: 0,
            failed: 0,
            first_failed: None,
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

    /// Return `true` when the rendered tuple does not represent every argument.
    ///
    /// A gap, a dropped tail or a shortened representation all mean the
    /// assembled message is a prefix rather than the whole tuple.
    fn has_omissions(&self) -> bool {
        self.is_incomplete() || self.first_failed.is_some() || self.truncated
    }

    /// Record that the argument at `index` produced no representation.
    fn record_failure(&mut self, index: usize) {
        self.failed += 1;
        self.first_failed.get_or_insert(index);
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
        shortened: Vec::with_capacity(total.min(limits.max_args)),
        total,
        failed: 0,
        first_failed: None,
        truncated: false,
        first_arg: None,
    };
    for (index, arg) in args.iter().take(limits.max_args).enumerate() {
        if index == 0 {
            rendered.first_arg = Some(arg.clone().unbind());
        }
        if index == 1 && types::is_exception_group(exception.py(), exception) {
            let members = group_members_text(&arg, limits);
            rendered.truncated |= members.text.truncated;
            rendered.failed = rendered.failed.saturating_add(members.failed);
            rendered.shortened.push(members.text.truncated);
            rendered.texts.push(members.text.text);
            continue;
        }
        let Some(text) = values::repr_bounded(&arg, limits.max_arg_bytes) else {
            rendered.record_failure(index);
            continue;
        };
        if text.failed {
            rendered.record_failure(index);
            continue;
        }
        rendered.truncated |= text.truncated;
        rendered.shortened.push(text.truncated);
        rendered.texts.push(text.text);
    }
    rendered
}

/// A bounded rendering of an exception group's member sequence.
struct GroupMembers {
    /// The represented members, inside the per-argument byte ceiling.
    text: BoundedText,
    /// Members whose representation raised and was replaced by a placeholder.
    failed: u32,
}

/// Render an exception group's member sequence without representing every member.
///
/// `repr` on the caller's sequence would call `__repr__` on every member,
/// including the ones the group walker drops, and build the whole text before
/// the byte ceiling could apply. Only the members the walker can retain are
/// represented, with one extra position read to prove that a tail exists. A
/// member's failing `__repr__` becomes a placeholder and is counted, so the
/// summary never grows past the member and byte ceilings.
fn group_members_text(value: &Bound<'_, PyAny>, limits: &CaptureLimits) -> GroupMembers {
    let placeholder = "<exception group members unavailable>";
    let Ok(iterator) = PyIterator::from_object(value) else {
        return GroupMembers {
            text: BoundedText::failure(placeholder, limits.max_arg_bytes),
            failed: 0,
        };
    };
    let probe = limits.max_group_width.saturating_add(1);
    let mut rendered = String::from("[");
    let mut failed = 0u32;
    let mut kept = 0usize;
    let mut tail = false;
    for (index, item) in iterator.take(probe).enumerate() {
        if index == limits.max_group_width {
            tail = true;
            break;
        }
        if kept > 0 {
            rendered.push_str(", ");
        }
        let Ok(item) = item else {
            failed = failed.saturating_add(1);
            rendered.push_str("<member unavailable>");
            kept += 1;
            continue;
        };
        match values::repr_bounded(&item, limits.max_arg_bytes) {
            Some(text) if !text.failed => rendered.push_str(&text.text),
            _ => {
                failed = failed.saturating_add(1);
                rendered.push_str("<repr conversion failed>");
            }
        }
        kept += 1;
    }
    if tail && kept > 0 {
        rendered.push_str(", ");
    }
    if tail {
        rendered.push_str(TRUNCATION_SUFFIX);
    }
    rendered.push(']');
    let was_shortened = tail || rendered.len() > limits.max_arg_bytes;
    let bounded = values::clamp(rendered, limits.max_arg_bytes);
    GroupMembers {
        text: BoundedText {
            truncated: bounded.truncated || was_shortened,
            ..bounded
        },
        failed,
    }
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
    for (index, text) in rendered.texts.into_iter().enumerate() {
        if !budget.charge_bytes(text.len()) {
            break;
        }
        // Shortening describes a representation the payload actually keeps;
        // an argument the charge refuses is reported as omitted instead.
        if rendered.shortened.get(index).copied().unwrap_or(false) {
            marker.args_truncated = true;
        }
        retained.push(text);
    }
    let unavailable = as_u32(rendered.total.saturating_sub(retained.len()));
    let omitted = unavailable.saturating_sub(rendered.failed);
    marker.args_omitted = marker.args_omitted.saturating_add(omitted);
    retained
}

mod build;
pub(super) use build::build_message;
#[cfg(test)]
use build::joined_args;

#[cfg(test)]
#[path = "message_tests.rs"]
mod tests;
