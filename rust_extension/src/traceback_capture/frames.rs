//! Bounded frame collection for tracebacks and live stacks.
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
use pyo3::types::{PyFrame, PyIterator, PyString, PyTraceback};

use crate::exception_schema::{PayloadTruncation, StackFrame};

use super::attrs::{as_u32, optional_attr};
use super::limits::{CaptureBudget, CaptureLimits};
use super::values;

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
        let Some(resolved) = resolve(&entry) else {
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

    let mut frames = Vec::with_capacity(retained.len());
    for descriptor in retained.into_iter().rev() {
        if budget.is_exhausted()
            || !append_frame(
                &linecache,
                &descriptor.frame,
                descriptor.lasti,
                descriptor.lineno,
                limits,
                budget,
                &mut frames,
            )
        {
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
    let mut frames = Vec::with_capacity(limits.max_frames);
    let mut current = Some(head);
    while let Some(entry) = current {
        let Some(resolved) = resolve(&entry) else {
            break;
        };
        current = resolved.following;
        visited += 1;
        if visited > limits.max_frames || budget.is_exhausted() {
            omitted += 1;
            continue;
        }
        if !append_frame(
            &linecache,
            &resolved.frame,
            resolved.lasti,
            resolved.lineno,
            limits,
            budget,
            &mut frames,
        ) {
            omitted += 1;
        }
    }
    marker.frames_omitted = marker.frames_omitted.saturating_add(as_u32(omitted));

    // `traceback.format_stack()` renders no column underline, and
    // `traceback.extract_stack()` leaves `colno`/`end_colno` unset. Keeping the
    // positions here would make the human-readable stack diverge from the
    // standard library, so a live stack reports the end line only.
    for frame in &mut frames {
        frame.colno = None;
        frame.end_colno = None;
    }

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

/// Convert one frame and append it, charging the bytes it retains.
///
/// Returns `false` when the frame was not retained, either because it could not
/// be built or because the aggregate budget refused it. Either way the caller
/// counts it as omitted, so the reported total matches the payload.
fn append_frame(
    linecache: &Bound<'_, PyAny>,
    frame: &Bound<'_, PyAny>,
    lasti: isize,
    lineno: u32,
    limits: &CaptureLimits,
    budget: &mut CaptureBudget,
    frames: &mut Vec<StackFrame>,
) -> bool {
    let Ok(converted) = build_frame(linecache, frame, lasti, lineno, limits) else {
        return false;
    };
    if !budget.charge_bytes(retained_bytes(&converted)) {
        return false;
    }
    frames.push(converted);
    true
}

/// Sum the retained text of a frame, used for aggregate accounting.
fn retained_bytes(frame: &StackFrame) -> usize {
    frame.filename.len() + frame.function.len() + frame.source_line.as_ref().map_or(0, String::len)
}

/// Build a schema frame from a live Python frame object.
fn build_frame(
    linecache: &Bound<'_, PyAny>,
    frame: &Bound<'_, PyAny>,
    lasti: isize,
    lineno: u32,
    limits: &CaptureLimits,
) -> PyResult<StackFrame> {
    let code = frame.getattr("f_code")?;
    let filename = bounded_text(&code.getattr("co_filename")?, limits.max_filename_bytes);
    let function = bounded_text(&code.getattr("co_name")?, limits.max_function_bytes);
    let (end_lineno, colno, end_colno) = code_positions(&code, lasti);

    Ok(StackFrame {
        source_line: lookup_source_line(linecache, &filename, lineno, limits.max_source_line_bytes),
        filename,
        lineno,
        // CPython's own `FrameSummary` substitutes the start line when the end
        // line is unavailable, so consumers always see a complete span.
        end_lineno: Some(end_lineno.unwrap_or(lineno)),
        colno,
        end_colno,
        function,
        // Capture never enables local-variable collection.
        locals: None,
    })
}

/// Read the end-line and column span for the instruction at `lasti`.
///
/// Returns three `None`s when the code object carries no position table, which
/// happens for code compiled without `co_positions` support.
fn code_positions(
    code: &Bound<'_, PyAny>,
    lasti: isize,
) -> (Option<u32>, Option<u32>, Option<u32>) {
    const NO_POSITION: (Option<u32>, Option<u32>, Option<u32>) = (None, None, None);
    if lasti < 0 {
        return NO_POSITION;
    }
    let Ok(positions) = code.call_method0("co_positions") else {
        return NO_POSITION;
    };
    let Ok(mut iterator) = PyIterator::from_object(&positions) else {
        return NO_POSITION;
    };
    // `co_positions()` yields one entry per instruction pair, which is the same
    // index CPython's own traceback walk uses.
    let Some(Ok(entry)) = iterator.nth((lasti / 2) as usize) else {
        return NO_POSITION;
    };
    (
        optional_u32(&entry, 1),
        optional_u32(&entry, 2),
        optional_u32(&entry, 3),
    )
}

/// Extract one `co_positions()` field, treating `None` as absent.
fn optional_u32(entry: &Bound<'_, PyAny>, index: usize) -> Option<u32> {
    entry.get_item(index).ok()?.extract::<u32>().ok()
}

/// Resolve a frame's source line through `linecache`.
///
/// Mirrors `traceback.FrameSummary.line`: the first physical line of the span,
/// stripped, or `None` when the source is unavailable. Code compiled from a
/// string (`exec`, `python -c`, a REPL) has no linecache entry and therefore
/// reports no source line.
fn lookup_source_line(
    linecache: &Bound<'_, PyAny>,
    filename: &str,
    lineno: u32,
    limit: usize,
) -> Option<String> {
    let raw = linecache.call_method1("getline", (filename, lineno)).ok()?;
    let raw = raw.cast::<PyString>().ok()?;
    let raw = values::bounded_string(raw, limit).text;
    let first_line = raw
        .split_once('\n')
        .map_or(raw.as_str(), |(line, _)| line)
        .trim();
    if first_line.is_empty() {
        return None;
    }
    Some(first_line.to_string())
}

/// Extract a string attribute, shortened to `limit` bytes.
fn bounded_text(value: &Bound<'_, PyAny>, limit: usize) -> String {
    let Ok(text) = value.cast::<PyString>() else {
        // `co_filename` and `co_name` are always strings; anything else is a
        // synthetic code object with nothing useful to report.
        return String::new();
    };
    values::bounded_string(text, limit).text
}

/// Read a `u32` attribute, treating a missing or negative value as absent.
fn extract_u32(obj: &Bound<'_, PyAny>, attr: &str) -> Option<u32> {
    obj.getattr(attr).ok()?.extract::<u32>().ok()
}

/// Read an `isize` attribute, treating a missing value as absent.
fn extract_isize(obj: &Bound<'_, PyAny>, attr: &str) -> Option<isize> {
    obj.getattr(attr).ok()?.extract::<isize>().ok()
}
