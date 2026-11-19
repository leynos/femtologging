//! Bounded frame collection for tracebacks and live stacks.
//!
//! This module walks the chain and decides which frames to keep; converting a
//! kept frame into a schema value lives in [`super::frame_build`]. Splitting the
//! two keeps each file under the module-size ceiling without mixing chain
//! traversal with the attribute reads that produce a payload.
//!
//! Frames are built from real Python frame objects rather than from a
//! `traceback.FrameSummary`, so the traversal stays under our own control: the
//! end-line and column span come from the code object's public `co_positions()`
//! stream, and the source line comes from `linecache`.
//!
//! # Two chain shapes, one walk
//!
//! A `__traceback__` chain and a live stack are both linked lists, but they
//! differ in two ways that would otherwise be duplicated here. A traceback node
//! is not a frame — it *describes* one through `tb_frame` — and its `tb_next`
//! runs inwards, while a live frame's `f_back` runs outwards. [`resolve`] folds
//! both shapes into one [`ChainEntry`], so the collectors below differ only in
//! the order they want their results in.
//!
//! # Only the retained frames are converted
//!
//! Both collectors retain the *innermost* [`CaptureLimits::max_frames`] frames,
//! so the frames nearest the failure survive. Tracebacks run outermost first,
//! therefore a fixed-capacity ring keeps the newest entries without converting
//! discarded frames or allocating in proportion to traceback depth.
//!
//! `locals` is always `None`. Rendering a frame's locals runs arbitrary
//! `__repr__` code for every binding in scope, which would be the largest
//! unpriced cost left in a frame, so capture never enables local collection.

use std::collections::VecDeque;

use pyo3::prelude::*;
use pyo3::types::{PyFrame, PyTraceback};

use crate::exception_schema::{PayloadTruncation, StackFrame};

use super::attrs::{as_u32, optional_attr};
use super::frame_build::{FrameRequest, PositionSource, PreparedSources, append_frame};
use super::limits::{CaptureBudget, CaptureLimits};

/// One entry of a traceback or frame chain, resolved to the frame it describes.
struct ChainEntry<'py> {
    /// The frame this entry stands for.
    frame: Bound<'py, PyAny>,
    /// Line number of the entry, `0` when the object does not report one.
    lineno: u32,
    /// Bytecode index used to read the code object's position table.
    lasti: isize,
    /// The next entry the chain links to, if any.
    following: Option<Bound<'py, PyAny>>,
}

/// A frame retained while the traceback chain is scanned.
struct FrameDescriptor<'py> {
    /// The frame to convert after the innermost window is known.
    frame: Bound<'py, PyAny>,
    /// Source line number associated with the frame.
    lineno: u32,
    /// Bytecode index used to read the code object's position table.
    lasti: isize,
}

/// Resolve one chain entry, whether it is a traceback node or a live frame.
///
/// Returns `None` for an object that is neither, which ends the walk rather than
/// inventing an entry for it. The type is checked before any attribute is read:
/// an `exc_info` tuple is caller-supplied, so a lookalike object that reports a
/// `tb_next` back to itself would otherwise keep the walk spinning while the
/// GIL is held. A real traceback cannot form such a loop, because CPython
/// rejects a cyclic `tb_next` assignment, and a real frame chain is finite.
fn resolve<'py>(entry: &Bound<'py, PyAny>) -> Option<ChainEntry<'py>> {
    if let Ok(traceback) = entry.cast::<PyTraceback>() {
        // A traceback node describes the frame it was raised in. `tb_next`
        // links inwards, towards the frame where the exception was raised.
        return Some(ChainEntry {
            frame: traceback.getattr("tb_frame").ok()?,
            lineno: extract_u32(entry, "tb_lineno").unwrap_or(0),
            lasti: extract_isize(entry, "tb_lasti").unwrap_or(-1),
            following: optional_attr(entry, "tb_next"),
        });
    }
    entry.cast::<PyFrame>().ok()?;
    // A live frame describes itself. `f_back` links outwards, towards the
    // frame's caller.
    Some(ChainEntry {
        frame: entry.clone(),
        lineno: extract_u32(entry, "f_lineno").unwrap_or(0),
        lasti: extract_isize(entry, "f_lasti").unwrap_or(-1),
        following: optional_attr(entry, "f_back"),
    })
}

/// Build the frames of a traceback chain, keeping the innermost `max_frames`.
///
/// `traceback` is the head of a `tb_next` chain, which runs outermost first, so
/// the frames dropped are the earliest ones and the result stays in
/// outermost-first order.
pub(super) fn collect_traceback_frames(
    py: Python<'_>,
    traceback: Option<&Bound<'_, PyAny>>,
    limits: &CaptureLimits,
    budget: &mut CaptureBudget,
    marker: &mut PayloadTruncation,
) -> Vec<StackFrame> {
    let Some(head) = traceback else {
        return Vec::new();
    };
    let Ok(linecache) = py.import("linecache") else {
        return Vec::new();
    };

    let mut retained = VecDeque::with_capacity(limits.max_frames);
    let mut omitted = 0usize;
    let mut current = Some(head.clone());
    while let Some(entry) = current {
        // Every node visited costs one unit of the visit allowance, so a
        // caller-supplied chain is bounded rather than followed to its full
        // length. The ring keeps the innermost window; extra visits past it
        // still add to the omission count while the allowance lasts.
        if !budget.charge_visit() {
            // The tail is unvisited, so only the fact that something was left is
            // recorded, not an exact count.
            omitted = omitted.saturating_add(1);
            break;
        }
        let Some(resolved) = resolve(&entry) else {
            // A link that is neither a traceback nor a frame ends the walk.
            // There is no frame to count as omitted: the object was never one.
            break;
        };
        current = resolved.following;
        if retained.len() == limits.max_frames {
            retained.pop_front();
            omitted += 1;
        }
        if limits.max_frames > 0 {
            retained.push_back(FrameDescriptor {
                frame: resolved.frame,
                lineno: resolved.lineno,
                lasti: resolved.lasti,
            });
        }
    }

    let mut prepared = PreparedSources::default();
    let mut frames = Vec::with_capacity(retained.len());
    for descriptor in retained.into_iter().rev() {
        let request = FrameRequest {
            linecache: &linecache,
            frame: &descriptor.frame,
            lasti: descriptor.lasti,
            lineno: descriptor.lineno,
            position_source: PositionSource::Instruction,
            prepared: &mut prepared,
        };
        if budget.is_exhausted() || !append_frame(request, limits, budget, &mut frames) {
            omitted += 1;
        }
    }
    frames.reverse();
    marker.frames_omitted = marker.frames_omitted.saturating_add(as_u32(omitted));
    frames
}

/// Build the frames of the caller's live stack, keeping the innermost frames.
///
/// The stack is walked innermost first through `f_back`, then reversed, so the
/// result is ordered oldest first — the order `traceback.extract_stack()`
/// produces and the text formatter expects.
pub(super) fn collect_stack_frames(
    py: Python<'_>,
    limits: &CaptureLimits,
    budget: &mut CaptureBudget,
    marker: &mut PayloadTruncation,
) -> PyResult<Vec<StackFrame>> {
    let linecache = py.import("linecache")?;
    let Some(head) = caller_frame(py) else {
        return Ok(Vec::new());
    };

    let mut omitted = 0usize;
    let mut visited = 0usize;
    let mut prepared = PreparedSources::default();
    let mut frames = Vec::with_capacity(limits.max_frames);
    let mut current = Some(head);
    while let Some(entry) = current {
        let Some(resolved) = resolve(&entry) else {
            break;
        };
        current = resolved.following;
        // The walk starts at the innermost frame, so once the window or the
        // budget is full there is nothing left to convert. The current frame is
        // dropped, and the rest of the stack is left unresolved: the count
        // records that a tail was omitted without claiming how long it is.
        if visited == limits.max_frames || budget.is_exhausted() {
            omitted += 1;
            break;
        }
        visited += 1;
        let request = FrameRequest {
            linecache: &linecache,
            frame: &resolved.frame,
            lasti: resolved.lasti,
            lineno: resolved.lineno,
            position_source: PositionSource::LineOnly,
            prepared: &mut prepared,
        };
        if !append_frame(request, limits, budget, &mut frames) {
            omitted += 1;
        }
    }
    marker.frames_omitted = marker.frames_omitted.saturating_add(as_u32(omitted));

    // Collected innermost first; callers expect the outermost frame first.
    frames.reverse();
    Ok(frames)
}

/// Return the Python frame that called into the extension, if there is one.
///
/// `sys._getframe()` is called without a depth argument because the `inspect`
/// equivalent is not usable from here: `inspect.currentframe()` is
/// `sys._getframe(1)`, which counts the *Python* frames above its own, and a
/// caller that is Rust or C contributes no frame to count. Asking for depth `1`
/// therefore skips past the extension entirely and raises `ValueError` unless
/// the application happens to be several frames deep.
///
/// A process with no Python frame beneath the extension — a Rust unit test, or a
/// C caller — has no stack to capture, and the same `ValueError` is how that
/// case reports itself. It is a missing stack rather than a failure, so the
/// capture comes back empty.
fn caller_frame(py: Python<'_>) -> Option<Bound<'_, PyAny>> {
    let sys = py.import("sys").ok()?;
    let frame = sys.call_method1("_getframe", (0,)).ok()?;
    (!frame.is_none()).then_some(frame)
}

/// Read a `u32` attribute, treating a missing or negative value as absent.
fn extract_u32(obj: &Bound<'_, PyAny>, attr: &str) -> Option<u32> {
    obj.getattr(attr).ok()?.extract::<u32>().ok()
}

/// Read an `isize` attribute, treating a missing value as absent.
fn extract_isize(obj: &Bound<'_, PyAny>, attr: &str) -> Option<isize> {
    obj.getattr(attr).ok()?.extract::<isize>().ok()
}
