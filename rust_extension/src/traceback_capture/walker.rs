//! Budgeted traversal of a real Python exception graph.
//!
//! The walker replaces `traceback.TracebackException`. It reads the exception's
//! own attributes, so the traversal is bounded by [`CaptureLimits`] rather than
//! by whatever the object graph happens to contain, and every conversion that
//! can run user code is contained by [`super::values`].
//!
//! # Traversal shape
//!
//! A node contributes its type, message, frames, arguments, notes, cause,
//! context and — for an exception group — its children, in that order. Cause
//! and context links consume [`CaptureLimits::max_chain_depth`], group nesting
//! consumes [`CaptureLimits::max_group_depth`], every visited node consumes one
//! unit of [`CaptureLimits::max_nodes`], and retained text is charged against
//! [`CaptureLimits::max_aggregate_bytes`].
//!
//! Recursion is safe here because both depth ceilings bound it: following a
//! link costs one of the two counters, so the deepest possible chain of Rust
//! frames is `max_chain_depth + max_group_depth` long.
//!
//! # Markers are local, exhaustion is global
//!
//! Each node carries a marker for the omissions *it* produced, so a reader can
//! tell which part of a chain was cut. `budget_exhausted` is the exception: it
//! describes the capture as a whole, so only the root payload carries it.
//!
//! Clipped identifiers (type name, module, filename, function) are deliberately
//! not reported. A clipped identifier already ends in the visible `...` marker,
//! and adding a flag would push every format-level detail into the payload.
//!
//! # Descendants retain no arguments or notes
//!
//! Only the exception handed to capture keeps `args_repr` and `notes`; every
//! descendant reports empty fields there, because the text formatter prints
//! neither. A descendant's *message* can still be assembled from its arguments —
//! a multi-argument exception's message is its arguments' `repr` — so argument
//! rendering is bounded by `max_args` and `max_arg_bytes` at every node, and the
//! resulting `repr` calls are counted in the same per-node budget wherever they
//! happen. Descendants report their message, type, frames, links and children in
//! full.

use std::collections::HashSet;

use crate::exception_schema::{EXCEPTION_SCHEMA_VERSION, ExceptionPayload, PayloadTruncation};
use pyo3::prelude::*;

use super::attrs::{
    Link, as_u32, bool_attr, bounded_sequence, identity, traceback_attribute, type_identity,
};
use super::fallback::minimal_payload;
use super::frames::collect_traceback_frames;
use super::limits::{CaptureBudget, CaptureLimits};
use super::message;
use super::notes;
use super::recursion::Recursion;
use super::types::{is_exception, is_exception_group};

/// Capture one exception and everything reachable from it within `limits`.
///
/// `traceback` overrides the exception's own `__traceback__`. Callers pass the
/// traceback supplied in an `exc_info` tuple, which can differ from the
/// attribute the exception still carries.
pub(super) fn capture_exception_payload(
    py: Python<'_>,
    exception: &Bound<'_, PyAny>,
    traceback: Option<&Bound<'_, PyAny>>,
    limits: &CaptureLimits,
) -> ExceptionPayload {
    let mut walker = Walker {
        py,
        limits,
        budget: CaptureBudget::new(limits),
        active: HashSet::new(),
        exhausted: false,
    };
    let root_has_budget = walker.budget.charge_node();
    let mut payload = if root_has_budget {
        walker.visit(exception, traceback, false, Recursion::root())
    } else {
        walker.exhausted = true;
        minimal_payload(exception, limits)
    };
    // Exhaustion describes the capture as a whole, so only the root payload
    // carries it; a descendant's marker stays local to that node.
    let mut marker = payload.truncation.take().unwrap_or_default();
    marker.budget_exhausted = walker.exhausted;
    payload.truncation = marker.into_option();
    payload
}

/// State threaded through one traversal.
struct Walker<'a> {
    /// Interpreter handle the traversal runs under.
    py: Python<'a>,
    /// Ceilings the payload must respect.
    limits: &'a CaptureLimits,
    /// Aggregate allowance shared by every node.
    budget: CaptureBudget,
    /// Identities on the current descent path, used to break cycles.
    active: HashSet<usize>,
    /// Whether any step recorded that the aggregate budget ran out.
    exhausted: bool,
}

impl Walker<'_> {
    /// Visit one exception node and return its payload.
    ///
    /// The returned payload carries a marker for this node's own omissions; the
    /// caller attaches the capture-wide `budget_exhausted` flag at the root.
    fn visit(
        &mut self,
        exception: &Bound<'_, PyAny>,
        traceback: Option<&Bound<'_, PyAny>>,
        traceback_read_failed: bool,
        at: Recursion,
    ) -> ExceptionPayload {
        // Identity is tracked for the active descent path only, so a group that
        // lists the same exception twice captures both positions while an edge
        // back to an ancestor is still rejected.
        let identity = identity(exception);
        self.active.insert(identity);
        let mut marker = PayloadTruncation::default();
        if traceback_read_failed {
            // The exception has a traceback attribute that raised, so its
            // frames are missing because the read failed, not because it had
            // none. Record the loss on this node and keep its other fields.
            marker.failed_elements = marker.failed_elements.saturating_add(1);
        }
        let (type_name, module) = type_identity(exception, self.limits);
        let (type_name, type_was_shortened) = self.budget.retain_text(type_name);
        self.exhausted |= type_was_shortened;
        let module = module.and_then(|module| {
            let (module, was_shortened) = self.budget.retain_text(module);
            self.exhausted |= was_shortened;
            (!module.is_empty()).then_some(module)
        });

        // Arguments are read before the message because a multi-argument
        // exception's message is assembled from them — see
        // [`message::build_message`]. Reading them is bounded by `max_args` and
        // `max_arg_bytes` and is not charged to the aggregate budget, which
        // keeps the documented spend order intact.
        let rendered_args =
            message::render_args(exception, self.limits, self.budget.remaining_bytes() > 0);
        rendered_args.report_failures(&mut marker);
        let message = self.retain_message(exception, &rendered_args, &mut marker);
        let frames = collect_traceback_frames(
            self.py,
            traceback,
            self.limits,
            &mut self.budget,
            &mut marker,
        );
        let args_repr = self.retain_args(at, rendered_args, &mut marker);
        let notes = self.retain_notes(exception, at, &mut marker);

        let (cause, retained_cause) =
            self.retain_link(exception, Link::Cause, at, &mut marker, None);
        // The context edge only checks for a duplicate when the cause edge kept
        // the same exception; otherwise the context is the only record of it.
        let (context, _) = self.retain_link(
            exception,
            Link::Context,
            at,
            &mut marker,
            retained_cause.as_ref(),
        );
        let exceptions = self.retain_group(exception, at, &mut marker);
        self.exhausted |= self.budget.is_exhausted();
        self.active.remove(&identity);

        ExceptionPayload {
            schema_version: EXCEPTION_SCHEMA_VERSION,
            type_name,
            module,
            message,
            args_repr,
            notes,
            frames,
            cause,
            context,
            suppress_context: bool_attr(exception, "__suppress_context__").unwrap_or(false),
            exceptions,
            truncation: marker.into_option(),
        }
    }

    /// Build the exception's message text, clamped to the message ceiling.
    ///
    /// Delegates to [`message::build_message`], which reads a multi-argument
    /// exception's message from the representations already taken rather than
    /// calling `str` and running every argument's `__repr__` a second time.
    fn retain_message(
        &mut self,
        exception: &Bound<'_, PyAny>,
        args: &message::RenderedArgs,
        marker: &mut PayloadTruncation,
    ) -> String {
        let remaining = self.budget.remaining_bytes();
        if remaining == 0 || self.limits.max_message_bytes == 0 {
            marker.message_truncated = true;
            self.exhausted |= remaining == 0;
            return String::new();
        }
        let message_limit = self.limits.max_message_bytes.min(remaining);
        let text = message::build_message(exception, args, message_limit);
        if text.failed {
            marker.failed_elements = marker.failed_elements.saturating_add(1);
        }
        marker.message_truncated |= text.truncated;
        // When the aggregate allowance is smaller than the message ceiling, a
        // shortened message means the budget ran out even though the clipped
        // text now fits the bytes that remain. `retain_text` cannot report that
        // second clipping, so the comparison is made here.
        if text.clipped && message_limit < self.limits.max_message_bytes {
            self.exhausted = true;
        }
        let (retained, aggregate_truncated) = self.budget.retain_text(text.text);
        marker.message_truncated |= aggregate_truncated;
        self.exhausted |= aggregate_truncated;
        retained
    }

    /// Retain the root's argument representations, charging them to the budget.
    ///
    /// Only the root keeps them: a descendant's `args_repr` is empty by contract,
    /// so retaining one would add text no formatter prints. Descendant
    /// conversion failures are recorded while their messages are built, but the
    /// argument representations themselves are discarded.
    fn retain_args(
        &mut self,
        at: Recursion,
        rendered: message::RenderedArgs,
        marker: &mut PayloadTruncation,
    ) -> Vec<String> {
        if !at.is_root {
            return Vec::new();
        }
        marker.args_truncated |= rendered.truncated;
        message::retain_args(rendered, &mut self.budget, marker)
    }

    /// Retain the root's `__notes__`, charging them to the budget.
    ///
    /// Only the root keeps notes: the text formatter prints a descendant's
    /// message, frames and links but never its notes, so collecting them would
    /// only run more user code for text nothing reads.
    fn retain_notes(
        &mut self,
        exception: &Bound<'_, PyAny>,
        at: Recursion,
        marker: &mut PayloadTruncation,
    ) -> Vec<String> {
        if !at.is_root {
            return Vec::new();
        }
        let (notes, exhausted) =
            notes::retain_notes(exception, self.limits, &mut self.budget, marker);
        self.exhausted |= exhausted;
        notes
    }

    /// Follow one cause or context link, or record why it was dropped.
    ///
    /// `retained_cause` is the object the cause edge captured, if any. Only then
    /// is a shared context redundant; a cause that was itself dropped leaves the
    /// context as the only record of that exception, so it is still followed or
    /// reported. The retained object — rather than a re-read of `__cause__` — is
    /// what the duplicate check compares, because the attribute can change
    /// between the two edges.
    ///
    /// Returns the payload, when one was built, together with the linked object
    /// so the caller can hand it to the next edge.
    fn retain_link<'py>(
        &mut self,
        exception: &Bound<'py, PyAny>,
        link: Link,
        at: Recursion,
        marker: &mut PayloadTruncation,
        retained_cause: Option<&Bound<'py, PyAny>>,
    ) -> (Option<Box<ExceptionPayload>>, Option<Bound<'py, PyAny>>) {
        let linked = match exception.getattr(link.attribute()) {
            Ok(linked) if !linked.is_none() => linked,
            Ok(_) => return (None, None),
            Err(_) => {
                marker.failed_elements = marker.failed_elements.saturating_add(1);
                link.mark_omitted(marker);
                return (None, None);
            }
        };
        // A property can return any object at all. Following one would build an
        // "exception" payload for something that is not an exception, so an
        // invalid target is reported as both a failed read and a dropped link.
        if !is_exception(self.py, &linked) {
            marker.failed_elements = marker.failed_elements.saturating_add(1);
            link.mark_omitted(marker);
            return (None, None);
        }
        // `raise X from e` inside `except ... as e` sets `__cause__` and
        // `__context__` to the same exception. When the cause edge already carried
        // it, the context edge adds nothing and leaving it out loses no data;
        // marking it omitted would put a marker on every such payload. The check
        // runs before the chain-depth test so a shared context at the ceiling is
        // also not reported as lost.
        if link.is_duplicate_context(retained_cause, &linked) {
            return (None, None);
        }
        if at.chain_depth >= self.limits.max_chain_depth {
            link.mark_omitted(marker);
            self.exhausted = true;
            return (None, None);
        }
        // A chain that points back at an exception on the active path would
        // otherwise be followed forever, so the edge is simply dropped. That is a
        // cycle in the object graph rather than a spent allowance, so the budget is
        // left as it was found.
        if self.active.contains(&identity(&linked)) {
            link.mark_omitted(marker);
            return (None, None);
        }
        if !self.budget.charge_node() {
            link.mark_omitted(marker);
            self.exhausted = true;
            return (None, None);
        }
        let (traceback, read_failed) = traceback_attribute(&linked);
        let payload = self.visit(&linked, traceback.as_ref(), read_failed, at.through_link());
        (Some(Box::new(payload)), Some(linked))
    }

    /// Collect an exception group's children within the width and depth limits.
    fn retain_group(
        &mut self,
        exception: &Bound<'_, PyAny>,
        at: Recursion,
        marker: &mut PayloadTruncation,
    ) -> Vec<ExceptionPayload> {
        if !is_exception_group(self.py, exception) {
            return Vec::new();
        }
        let children = match exception.getattr("exceptions") {
            Ok(children) if !children.is_none() => children,
            Ok(_) | Err(_) => {
                marker.failed_elements = marker.failed_elements.saturating_add(1);
                marker.exceptions_omitted = 1;
                marker.members_truncated = true;
                return Vec::new();
            }
        };
        if at.group_depth >= self.limits.max_group_depth {
            let bounded = bounded_sequence(&children, 0);
            marker.failed_elements =
                marker
                    .failed_elements
                    .saturating_add(if bounded.failed { 1 } else { 0 });
            marker.exceptions_omitted = as_u32(bounded.omitted);
            marker.members_truncated = bounded.omitted > 0;
            self.exhausted |= bounded.omitted > 0;
            return Vec::new();
        }

        let bounded = bounded_sequence(&children, self.limits.max_group_width);
        marker.failed_elements =
            marker
                .failed_elements
                .saturating_add(if bounded.failed { 1 } else { 0 });
        let mut omitted = bounded.omitted;
        let mut retained = Vec::with_capacity(bounded.items.len());
        let mut member_cut = false;
        for child in &bounded.items {
            if !is_exception(self.py, child) {
                omitted = omitted.saturating_add(1);
                marker.failed_elements = marker.failed_elements.saturating_add(1);
                continue;
            }
            if self.active.contains(&identity(child)) || !self.budget.charge_node() {
                omitted = omitted.saturating_add(1);
                self.exhausted |= self.budget.is_exhausted();
                continue;
            }
            let (traceback, read_failed) = traceback_attribute(child);
            let payload = self.visit(child, traceback.as_ref(), read_failed, at.through_group());
            member_cut |= payload.truncation.is_some();
            retained.push(payload);
        }
        marker.exceptions_omitted = as_u32(omitted);
        // A group counts as shortened when a member was cut or when a member was
        // dropped outright. `members_truncated` is the only such signal at the
        // group level: each member reports its own omissions on itself, and a
        // dropped member has no payload left to report them.
        marker.members_truncated |= member_cut || omitted > 0 || bounded.failed;
        retained
    }
}
