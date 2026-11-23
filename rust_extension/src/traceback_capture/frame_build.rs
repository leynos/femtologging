//! Conversion of one retained Python frame into a schema frame.
//!
//! [`super::frames`] decides *which* frames to keep; this module turns a kept
//! frame into a [`StackFrame`]. The split keeps each file under the module-size
//! ceiling without mixing chain traversal with the attribute reads that produce
//! a payload.
//!
//! A frame is built from real Python frame and code objects rather than from a
//! `traceback.FrameSummary`, so the traversal stays under our own control: the
//! end-line and column span come from the code object's public `co_positions()`
//! stream, and the source line comes from `linecache`.
//!
//! # Bounded work per frame
//!
//! `co_positions()` yields one entry per instruction, so a generated function
//! with a large code object could otherwise make one frame cost thousands of
//! iterations on the logging thread. [`MAX_POSITION_SCAN`] caps the scan and
//! falls back to the line number the chain reported.
//!
//! Source lookup mirrors `traceback.StackSummary`: it primes and refreshes
//! `linecache` once per retained filename, selects and trims the displayed line,
//! and only then applies the byte ceiling.

use std::collections::HashSet;

use pyo3::prelude::*;
use pyo3::types::{PyIterator, PyString, PyStringMethods};

use crate::exception_schema::StackFrame;

use super::limits::{CaptureBudget, CaptureLimits, truncate_to_limit};
use super::values;

/// Largest instruction index whose position entry is read.
///
/// A frame's `lasti` names an instruction within its code object, so a scan is
/// already bounded by the function's size. Generated functions can still be
/// large and capture runs while the GIL is held, so the scan is capped: past
/// this point the frame keeps its line number without a column span.
const MAX_POSITION_SCAN: usize = 4096;

/// Whether a frame's instruction positions are read.
#[derive(Clone, Copy)]
pub(super) enum PositionSource {
    /// Read the code object's position table at the frame's instruction.
    Instruction,
    /// Report the line only. A live stack has no exception instruction to
    /// point at, and the standard library's stack summaries carry no columns.
    LineOnly,
}

/// Line and column data for one instruction.
#[derive(Default)]
struct Positions {
    /// Line the instruction starts on.
    start_lineno: Option<u32>,
    /// Line the instruction ends on.
    end_lineno: Option<u32>,
    /// One-based first column of the instruction.
    colno: Option<u32>,
    /// One-based column just past the instruction.
    end_colno: Option<u32>,
}

impl Positions {
    /// No position data, used when the code object has no table or is skipped.
    const NONE: Self = Self {
        start_lineno: None,
        end_lineno: None,
        colno: None,
        end_colno: None,
    };
}

/// Filenames whose source has already been primed and refreshed.
#[derive(Default)]
pub(super) struct PreparedSources {
    /// Identities of `co_filename` objects already prepared.
    seen: HashSet<usize>,
}

/// One frame to convert, together with the lookup state it needs.
pub(super) struct FrameRequest<'py, 'a> {
    /// Interpreter `linecache` module.
    pub(super) linecache: &'a Bound<'py, PyAny>,
    /// Frame object to read.
    pub(super) frame: &'a Bound<'py, PyAny>,
    /// Bytecode index used to read the code object's position table.
    pub(super) lasti: isize,
    /// Line number the chain reported for the frame.
    pub(super) lineno: u32,
    /// Whether the instruction's position entry should be read.
    pub(super) position_source: PositionSource,
    /// Sources already primed and refreshed during this capture.
    pub(super) prepared: &'a mut PreparedSources,
}

/// Convert one frame and append it, charging the bytes it retains.
///
/// Returns `false` when the frame was not retained, either because it could not
/// be built or because the aggregate budget refused it. Either way the caller
/// counts it as omitted, so the reported total matches the payload.
pub(super) fn append_frame(
    request: FrameRequest<'_, '_>,
    limits: &CaptureLimits,
    budget: &mut CaptureBudget,
    frames: &mut Vec<StackFrame>,
) -> bool {
    let Ok(converted) = build_frame(request, limits) else {
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
fn build_frame(request: FrameRequest<'_, '_>, limits: &CaptureLimits) -> PyResult<StackFrame> {
    let code = request.frame.getattr("f_code")?;
    let filename_value = code.getattr("co_filename")?;
    let filename = bounded_text(&filename_value, limits.max_filename_bytes);
    let function = bounded_text(&code.getattr("co_name")?, limits.max_function_bytes);
    let positions = match request.position_source {
        PositionSource::Instruction => code_positions(&code, request.lasti),
        PositionSource::LineOnly => Positions::NONE,
    };
    // The position entry and the source line must describe the same line. A
    // synthetic traceback can carry a `tb_lineno` that differs from the
    // instruction's start line, so the position value wins when it exists and
    // the chain's line number is the fallback.
    let lineno = positions.start_lineno.unwrap_or(request.lineno);

    Ok(StackFrame {
        source_line: lookup_source_line(
            request.linecache,
            request.frame,
            &filename_value,
            lineno,
            limits.max_source_line_bytes,
            request.prepared,
        ),
        filename,
        lineno,
        // CPython's own `FrameSummary` substitutes the start line when the end
        // line is unavailable, so consumers always see a complete span.
        end_lineno: Some(positions.end_lineno.unwrap_or(lineno)),
        colno: positions.colno,
        end_colno: positions.end_colno,
        function,
        // Capture never enables local-variable collection.
        locals: None,
    })
}

/// Read the line and column span for the instruction at `lasti`.
///
/// Returns [`Positions::NONE`] when the code object carries no position table,
/// which happens for code compiled without `co_positions` support, or when the
/// instruction index is beyond [`MAX_POSITION_SCAN`].
fn code_positions(code: &Bound<'_, PyAny>, lasti: isize) -> Positions {
    if lasti < 0 {
        return Positions::NONE;
    }
    let index = (lasti / 2) as usize;
    if index >= MAX_POSITION_SCAN {
        return Positions::NONE;
    }
    let Ok(positions) = code.call_method0("co_positions") else {
        return Positions::NONE;
    };
    let Ok(mut iterator) = PyIterator::from_object(&positions) else {
        return Positions::NONE;
    };
    // `co_positions()` yields one entry per instruction pair, which is the same
    // index CPython's own traceback walk uses.
    let Some(Ok(entry)) = iterator.nth(index) else {
        return Positions::NONE;
    };
    Positions {
        start_lineno: optional_u32(&entry, 0),
        end_lineno: optional_u32(&entry, 1),
        colno: optional_u32(&entry, 2),
        end_colno: optional_u32(&entry, 3),
    }
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
///
/// The original `co_filename` object is passed to `linecache`; the bounded copy
/// in the payload is only for display. A shortened path would make a real file
/// unfindable, or find a different file whose name shares the retained prefix.
///
/// The line is selected and trimmed before its byte ceiling is applied, so a
/// line that fits once its leading indentation and trailing newline are removed
/// is not replaced by a truncation marker. One extra code point is read so a
/// newline sitting exactly at the ceiling is still visible to the split.
fn lookup_source_line(
    linecache: &Bound<'_, PyAny>,
    frame: &Bound<'_, PyAny>,
    filename: &Bound<'_, PyAny>,
    lineno: u32,
    limit: usize,
    prepared: &mut PreparedSources,
) -> Option<String> {
    let Ok(filename) = filename.cast::<PyString>() else {
        return None;
    };
    prepare_source(linecache, frame, filename, prepared);
    if !source_is_bounded(filename.py(), linecache, filename) {
        return None;
    }
    let raw = linecache.call_method1("getline", (filename, lineno)).ok()?;
    let raw = raw.cast::<PyString>().ok()?;
    let trimmed = trim_short_line(raw);
    let text = match trimmed {
        Some(line) => line,
        // A line too long to trim safely is bounded first; a pathological
        // source line can then lose its statement to leading indentation, but
        // the scan stays inside the ceiling.
        None => values::bounded_string(raw, limit.saturating_add(1)).text,
    };
    let first_line = text
        .split_once('\n')
        .map_or(text.as_str(), |(line, _)| line)
        .trim();
    if first_line.is_empty() {
        return None;
    }
    Some(truncate_to_limit(first_line, limit).unwrap_or_else(|| first_line.to_string()))
}

/// Return `true` when `linecache` can supply this file without an unbounded read.
///
/// `linecache.getline` reads and caches a whole file on a miss, so a frame in a
/// file with a huge trailing section would cost the file's size on the logging
/// thread. A file already in `linecache.cache` has already paid that cost, so a
/// hit is free; otherwise the file's size is checked first and a source larger
/// than [`MAX_SOURCE_BYTES`] is omitted, leaving the frame's filename, function
/// and line number in place. A path that is not a regular file — an archive
/// member, say — also yields no source rather than an unbounded loader read.
fn source_is_bounded(
    py: Python<'_>,
    linecache: &Bound<'_, PyAny>,
    filename: &Bound<'_, PyString>,
) -> bool {
    if cached_source(linecache, filename) {
        return true;
    }
    let Ok(os) = py.import("os") else {
        return false;
    };
    let Ok(path) = os.getattr("path") else {
        return false;
    };
    path.call_method1("getsize", (filename,))
        .ok()
        .and_then(|size| size.extract::<u64>().ok())
        .is_some_and(|size| size <= MAX_SOURCE_BYTES)
}

/// Return `true` when `linecache` already holds this filename.
///
/// A cache hit has already paid for reading the source, so using it adds no
/// unbounded work of its own.
fn cached_source(linecache: &Bound<'_, PyAny>, filename: &Bound<'_, PyString>) -> bool {
    let Ok(cache) = linecache.getattr("cache") else {
        return false;
    };
    let Ok(cache) = cache.cast::<pyo3::types::PyDict>() else {
        return false;
    };
    cache.contains(filename).unwrap_or(false)
}

/// Largest source file whose line is read through `linecache`.
const MAX_SOURCE_BYTES: u64 = 1024 * 1024;

/// Longest source line copied in full so it can be trimmed before clamping.
const MAX_TRIMMED_LINE: usize = 8 * 1024;

/// Return the trimmed contents of a short source line.
///
/// `linecache` returns one physical line, but it can carry any amount of leading
/// indentation or trailing padding. Trimming first lets a statement that fits
/// once its indentation is removed survive the byte ceiling, which is what
/// `traceback.FrameSummary` displays. The trim is only attempted for a line
/// short enough that copying it whole is bounded; a longer line takes the
/// prefix path in the caller instead.
fn trim_short_line(raw: &Bound<'_, PyString>) -> Option<String> {
    let code_points = raw.code_point_len().ok()?;
    if code_points > MAX_TRIMMED_LINE {
        return None;
    }
    let py = raw.py();
    let strip = py.get_type::<PyString>().getattr("strip").ok()?;
    let stripped = strip.call1((raw,)).ok()?;
    let stripped = stripped.cast::<PyString>().ok()?;
    Some(stripped.to_string_lossy().into_owned())
}

/// Prime and refresh `linecache` for one source file, once per capture.
///
/// `lazycache` registers the frame's globals so a loader such as `zipimport`
/// can supply source for a module whose file is an archive member. `checkcache`
/// then drops a stale cached copy, matching `traceback.StackSummary`, which
/// refreshes every retained filename before reading any line.
fn prepare_source(
    linecache: &Bound<'_, PyAny>,
    frame: &Bound<'_, PyAny>,
    filename: &Bound<'_, PyString>,
    prepared: &mut PreparedSources,
) {
    if !prepared.seen.insert(filename.as_ptr() as usize) {
        return;
    }
    if let Ok(globals) = frame.getattr("f_globals") {
        let _ = linecache.call_method1("lazycache", (filename, globals));
    }
    let _ = linecache.call_method1("checkcache", (filename,));
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
