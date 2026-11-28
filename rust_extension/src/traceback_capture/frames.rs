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

use pyo3::exceptions::PyValueError;
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

/// The outcome of resolving one chain entry.
enum Resolved<'py> {
    /// A traceback node or live frame.
    Entry(ChainEntry<'py>),
    /// The object is neither, so the walk has reached its end.
    End,
    /// The object is a traceback, but a required attribute read raised, so a
    /// frame was lost rather than absent.
    Lost,
}

/// Resolve one chain entry, whether it is a traceback node or a live frame.
///
/// [`Resolved::End`] is returned for an object that is neither, which ends the
/// walk rather than inventing an entry for it. The type is checked before any
/// attribute is read: an `exc_info` tuple is caller-supplied, so a lookalike
/// object that reports a `tb_next` back to itself would otherwise keep the walk
/// spinning while the GIL is held. A real traceback cannot form such a loop,
/// because CPython rejects a cyclic `tb_next` assignment, and a real frame chain
/// is finite.
fn resolve<'py>(entry: &Bound<'py, PyAny>) -> Resolved<'py> {
    if let Ok(traceback) = entry.cast::<PyTraceback>() {
        // A traceback node describes the frame it was raised in. `tb_next`
        // links inwards, towards the frame where the exception was raised.
        let Ok(frame) = traceback.getattr("tb_frame") else {
            // An audit hook can reject the read even for a real traceback.
            return Resolved::Lost;
        };
        return Resolved::Entry(ChainEntry {
            frame,
            lineno: extract_u32(entry, "tb_lineno").unwrap_or(0),
            lasti: extract_isize(entry, "tb_lasti").unwrap_or(-1),
            following: optional_attr(entry, "tb_next"),
        });
    }
    if entry.cast::<PyFrame>().is_err() {
        return Resolved::End;
    }
    // A live frame describes itself. `f_back` links outwards, towards the
    // frame's caller.
    Resolved::Entry(ChainEntry {
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
///
/// The scan is bounded by the visit allowance. If it expires before the end of
/// the chain, the innermost frames — including the throw site — were never
/// reached, so the retained window is a tail of the *visited prefix* rather
/// than of the traceback. That case sets
/// [`PayloadTruncation::frames_omitted_is_lower_bound`], which tells consumers
/// the count is a minimum and the window may not reach the failure.
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
        // The chain exists but cannot be converted; an unmarked empty list
        // would read as "there were no frames".
        marker.frames_omitted = marker.frames_omitted.saturating_add(1);
        marker.frames_omitted_is_lower_bound = true;
        return Vec::new();
    };

    // A caller-supplied ceiling can be far larger than any real chain, so the
    // ring starts small and grows with the entries actually seen.
    let mut retained = VecDeque::with_capacity(limits.max_frames.min(8));
    let mut omitted = 0usize;
    let mut current = Some(head.clone());
    while let Some(entry) = current {
        // Every node visited costs one unit of the visit allowance, so a
        // caller-supplied chain is bounded rather than followed to its full
        // length. The ring keeps the innermost window; extra visits past it
        // still add to the omission count while the allowance lasts.
        if !budget.charge_visit() {
            // The tail is unvisited, so only the fact that something was left is
            // recorded, not an exact count. The visit failure is reported only
            // through `frames_omitted_is_lower_bound`; marking the budget
            // exhausted would also skip conversion of frames already retained.
            omitted = omitted.saturating_add(1);
            marker.frames_omitted_is_lower_bound = true;
            break;
        }
        let resolved = match resolve(&entry) {
            Resolved::Entry(resolved) => resolved,
            // A link that is neither a traceback nor a frame ends the walk.
            // There is no frame to count as omitted: the object was never one.
            Resolved::End => break,
            Resolved::Lost => {
                // The node was a traceback but its frame could not be read.
                omitted = omitted.saturating_add(1);
                marker.frames_omitted_is_lower_bound = true;
                break;
            }
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
    let head = match caller_frame(py) {
        Ok(Some(head)) => head,
        // No Python frame beneath the extension is a normal empty stack; a
        // raising audit hook is not, and must be reported rather than read as
        // "there was nothing to capture".
        Ok(None) => return Ok(Vec::new()),
        Err(_) => {
            marker.frames_omitted = marker.frames_omitted.saturating_add(1);
            marker.frames_omitted_is_lower_bound = true;
            return Ok(Vec::new());
        }
    };

    let mut omitted = 0usize;
    let mut visited = 0usize;
    let mut prepared = PreparedSources::default();
    let mut frames = Vec::with_capacity(limits.max_frames.min(8));
    let mut current = Some(head);
    while let Some(entry) = current {
        let resolved = match resolve(&entry) {
            Resolved::Entry(resolved) => resolved,
            Resolved::End => break,
            Resolved::Lost => {
                omitted += 1;
                marker.frames_omitted_is_lower_bound = true;
                break;
            }
        };
        current = resolved.following;
        // The walk starts at the innermost frame, so once the window or the
        // budget is full there is nothing left to convert. The current frame is
        // dropped, and the rest of the stack is left unresolved: the count
        // records that a tail was omitted without claiming how long it is.
        if visited == limits.max_frames || budget.is_exhausted() {
            // The remaining stack was not walked, so the count proves a tail
            // was omitted without saying how long it is.
            omitted += 1;
            marker.frames_omitted_is_lower_bound = true;
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
/// C caller — has no stack to capture. That case and a raising audit hook both
/// surface as errors from `_getframe`, so the two are distinguished by type:
/// `ValueError` is a missing stack, anything else is a lost one.
fn caller_frame(py: Python<'_>) -> PyResult<Option<Bound<'_, PyAny>>> {
    let sys = py.import("sys")?;
    match sys.call_method1("_getframe", (0,)) {
        Ok(frame) => Ok((!frame.is_none()).then_some(frame)),
        // The interpreter reports a shallow stack with an exact `ValueError`
        // whose sole argument is a fixed string. Reading the argument does not
        // run user code; comparing the type and text this way avoids calling a
        // subclass's `__str__` just to classify the failure.
        Err(error) if is_shallow_stack_error(&error, py) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Read a `u32` attribute, treating a missing or negative value as absent.
fn extract_u32(obj: &Bound<'_, PyAny>, attr: &str) -> Option<u32> {
    obj.getattr(attr).ok()?.extract::<u32>().ok()
}

/// Read an `isize` attribute, treating a missing value as absent.
fn extract_isize(obj: &Bound<'_, PyAny>, attr: &str) -> Option<isize> {
    obj.getattr(attr).ok()?.extract::<isize>().ok()
}

/// Return `true` when `error` is the interpreter's shallow-stack `ValueError`.
fn is_shallow_stack_error(error: &PyErr, py: Python<'_>) -> bool {
    if !error.get_type(py).is(py.get_type::<PyValueError>()) {
        return false;
    }
    let value = error.value(py);
    let Ok(base) = py
        .import("builtins")
        .and_then(|builtins| builtins.getattr("BaseException"))
    else {
        return false;
    };
    let Ok(descriptor) = base.getattr("args") else {
        return false;
    };
    let Ok(args) = descriptor.call_method1("__get__", (value, value.get_type())) else {
        return false;
    };
    let Ok(args) = args.cast::<pyo3::types::PyTuple>() else {
        return false;
    };
    args.len() == 1
        && args
            .get_item(0)
            .ok()
            .and_then(|arg| arg.extract::<String>().ok())
            .is_some_and(|text| text == "call stack is not deep enough")
}
