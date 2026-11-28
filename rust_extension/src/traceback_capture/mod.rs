//! Python traceback capture utilities.
//!
//! This module extracts exception and stack-trace data from Python and converts
//! it to the schema types in [`crate::exception_schema`]. All capture happens on
//! the caller thread while the GIL is held, ensuring worker threads stay
//! GIL-free.
//!
//! # Bounded capture
//!
//! Capture is a robustness boundary: the values being captured come from
//! application code, and rendering them runs that code's `__str__`, `__repr__`
//! and attribute lookups. Every traversal is therefore bounded by
//! [`CaptureLimits`], and every conversion is contained. The limits bound
//! traversal and retained data; they cannot pre-empt a single Python conversion
//! that blocks or allocates before returning, so such a call may still take
//! arbitrary time.
//!
//! Two failure families are deliberately kept apart:
//!
//! - **Input resolution** — an `exc_info` argument that is not a bool, an
//!   exception instance or a 3-tuple — still raises `TypeError`, because the
//!   caller made a programming error.
//! - **Traversal** — anything that goes wrong once a real exception has been
//!   accepted — degrades to a type-only payload carrying `budget_exhausted`
//!   rather than propagating, because logging must not fail on the way out of a
//!   failure.
//!
//! The second family includes a panic. Capture runs user code, so a panic here
//! would otherwise escape into the logging call and, from a `#[pymethods]`
//! entry point, into Python as a `PanicException`.
//!
//! # Usage
//!
//! The primary entry points are [`capture_exception`] for `exc_info` handling
//! and [`capture_stack`] for `stack_info=True` support.

use std::panic::{AssertUnwindSafe, catch_unwind};

use pyo3::prelude::*;
use pyo3::types::{PyBool, PyTuple};

use crate::exception_schema::{
    EXCEPTION_SCHEMA_VERSION, ExceptionPayload, PayloadTruncation, StackTracePayload,
};

// The traversal is an implementation detail of this module; only the entry
// points and the limit types are reachable from elsewhere in the crate.
mod attrs;
mod fallback;
mod frame_build;
mod frames;
mod limits;
mod message;
mod notes;
mod recursion;
mod types;
mod values;
mod walker;

pub(crate) use self::limits::{CaptureBudget, CaptureLimits};
#[cfg(test)]
pub(crate) use self::limits::{TRUNCATION_SUFFIX, TRUNCATION_SUFFIX_LEN, truncate_to_limit};

#[cfg(test)]
pub(crate) use self::counter::{capture_count, reset_capture_count};

/// Increment the test-only traversal counter when running under test.
#[cfg(test)]
fn note_capture() {
    counter::record_capture();
}

/// Compile the traversal counter away in production builds.
#[cfg(not(test))]
#[inline]
fn note_capture() {}

/// Where a capture's traceback comes from.
enum Traceback<'py> {
    /// The caller supplied the traceback explicitly, as in an `exc_info`
    /// triple; `None` means the caller supplied no traceback at all.
    Explicit(Option<Bound<'py, PyAny>>),
    /// Read the exception's own `__traceback__` during the guarded traversal.
    FromException,
}

/// How `exc_info` was supplied by the caller.
enum ExcInfoKind {
    /// `exc_info=True`: use the currently handled exception.
    Current,
    /// `exc_info=False` or `None`: capture nothing.
    Nothing,
    /// An explicit `(type, value, traceback)` triple.
    Tuple(Py<PyTuple>),
    /// An exception instance.
    Instance,
}

/// Capture exception information from Python's `exc_info` argument.
///
/// Handles the forms Python's `logging` accepts:
///
/// - `True`: use `sys.exc_info()` for the exception being handled
/// - an exception instance: capture it, together with its own `__traceback__`
/// - a 3-tuple `(type, value, traceback)`: use the value and traceback given
///
/// Returns `None` when `exc_info=True` but no exception is being handled.
///
/// # Errors
///
/// Returns [`pyo3::exceptions::PyTypeError`] when `exc_info` is none of the
/// accepted forms. Traversal failures never propagate: they degrade to a
/// type-only payload instead.
///
/// # Examples
///
/// ```rust,ignore
/// Python::attach(|py| {
///     let exc = py.eval(c"ValueError('bad input')", None, None)?;
///     let payload = capture_exception(py, &exc)?.expect("exception captured");
///     assert_eq!(payload.type_name, "ValueError");
///     Ok::<_, PyErr>(())
/// })?;
/// ```
pub fn capture_exception(
    py: Python<'_>,
    exc_info: &Bound<'_, PyAny>,
) -> PyResult<Option<ExceptionPayload>> {
    capture_exception_with_limits(py, exc_info, &CaptureLimits::default())
}

/// Validate `exc_info` without capturing any exception data.
///
/// Callers that defer capture until after a filter prefix has run use this to
/// report an invalid `exc_info` argument before filtering, so a filter can
/// never suppress the caller's programming error. Capture itself re-runs the
/// same classification and cannot fail for a shape this accepts.
///
/// # Errors
///
/// Returns [`pyo3::exceptions::PyTypeError`] when `exc_info` is none of the
/// accepted forms.
///
/// # Examples
///
/// ```rust,ignore
/// Python::attach(|py| {
///     let flag = py.True();
///     validate_exc_info(py, flag.as_any()).expect("True is valid");
///     Ok::<_, PyErr>(())
/// })?;
/// ```
pub fn validate_exc_info(py: Python<'_>, exc_info: &Bound<'_, PyAny>) -> PyResult<()> {
    classify_exc_info(py, exc_info).map(|_| ())
}

/// Capture exception information under explicit `limits`.
///
/// The entry point tests use to force truncation with small ceilings; the public
/// [`capture_exception`] applies [`CaptureLimits::default`]. Capture refreshes
/// the process-wide `linecache` and may read a regular source file up to 1 MiB
/// for a retained frame's source line.
///
/// # Errors
///
/// As for [`capture_exception`].
pub fn capture_exception_with_limits(
    py: Python<'_>,
    exc_info: &Bound<'_, PyAny>,
    limits: &CaptureLimits,
) -> PyResult<Option<ExceptionPayload>> {
    match classify_exc_info(py, exc_info)? {
        ExcInfoKind::Current => capture_current_exception(py, limits),
        ExcInfoKind::Nothing => Ok(None),
        ExcInfoKind::Tuple(tuple) => capture_from_tuple(py, tuple.bind(py), limits),
        // The traceback read runs user code, so it happens inside the guarded
        // traversal rather than beside it.
        ExcInfoKind::Instance => Ok(Some(capture_with_fallback(
            py,
            exc_info,
            Traceback::FromException,
            limits,
        ))),
    }
}

/// Capture the current call stack for `stack_info=True`.
///
/// Frames are ordered oldest first, matching both the text formatter's
/// expectations and the order `traceback.extract_stack()` produces. Filtering
/// infrastructure frames is left to the caller's frame filter, which is
/// configured separately from the capture limits.
///
/// # Errors
///
/// Returns an error only if the interpreter's own inspection modules cannot be
/// imported; an unusable frame is skipped rather than failing the capture.
pub fn capture_stack(py: Python<'_>) -> PyResult<StackTracePayload> {
    capture_stack_with_limits(py, &CaptureLimits::default())
}

/// Capture the current call stack under explicit `limits`.
///
/// The entry point tests use to force truncation with small ceilings; the public
/// [`capture_stack`] applies [`CaptureLimits::default`].
///
/// # Errors
///
/// As for [`capture_stack`].
pub fn capture_stack_with_limits(
    py: Python<'_>,
    limits: &CaptureLimits,
) -> PyResult<StackTracePayload> {
    note_capture();
    // Walking the stack reads frames the application can influence: an audit
    // hook on `sys._getframe` runs user code, and a panic there must not escape
    // into the logging call.
    let collected = catch_unwind(AssertUnwindSafe(|| {
        let mut budget = CaptureBudget::new(limits);
        let mut marker = PayloadTruncation::default();
        let frames = frames::collect_stack_frames(py, limits, &mut budget, &mut marker)?;
        Ok::<_, PyErr>((frames, budget, marker))
    }));

    match collected {
        Ok(Ok((frames, budget, mut marker))) => {
            if budget.is_exhausted() {
                marker.budget_exhausted = true;
            }
            Ok(StackTracePayload {
                schema_version: EXCEPTION_SCHEMA_VERSION,
                frames,
                truncation: marker.into_option(),
            })
        }
        // Only an interpreter-level import failure is reported; unusable
        // frames are skipped inside the collector.
        Ok(Err(error)) => Err(error),
        Err(_) => {
            let _ = PyErr::take(py);
            Ok(StackTracePayload {
                schema_version: EXCEPTION_SCHEMA_VERSION,
                frames: Vec::new(),
                truncation: Some(PayloadTruncation {
                    budget_exhausted: true,
                    ..Default::default()
                }),
            })
        }
    }
}

/// Capture `exc_value` with `traceback`, degrading on traversal failure.
///
/// The walker is written not to fail, but it converts values from application
/// code, where any conversion can raise or panic. Falling back to a type-only
/// payload keeps the logging call alive, which matters more here than
/// completeness: capture is what happens on the way out of a failure.
fn capture_with_fallback(
    py: Python<'_>,
    exc_value: &Bound<'_, PyAny>,
    traceback: Traceback<'_>,
    limits: &CaptureLimits,
) -> ExceptionPayload {
    note_capture();
    let captured = catch_unwind(AssertUnwindSafe(|| {
        // An absent traceback and a raising getter read differently: the
        // former is normal, the latter is a loss the payload must report. An
        // explicit `None` from the caller means "no traceback" and must not be
        // replaced by the exception's own attribute.
        let (traceback, read_failed) = match traceback {
            Traceback::Explicit(traceback) => (traceback, false),
            Traceback::FromException => attrs::traceback_attribute(exc_value),
        };
        walker::capture_exception_payload(py, exc_value, traceback.as_ref(), read_failed, limits)
    }));
    match captured {
        Ok(payload) => payload,
        Err(_) => {
            // A panic can leave an exception indicator set; clearing it stops a
            // later Python call from reporting an unrelated failure.
            let _ = PyErr::take(py);
            fallback::minimal_payload(exc_value, limits)
        }
    }
}

/// Capture the exception currently being handled, if any.
fn capture_current_exception(
    py: Python<'_>,
    limits: &CaptureLimits,
) -> PyResult<Option<ExceptionPayload>> {
    let sys = py.import("sys")?;
    let current = sys.call_method0("exc_info")?;
    let tuple = current.cast::<PyTuple>()?;
    // `exc_info()` returns `(type, value, traceback)`, each `None` when no
    // exception is being handled.
    if tuple.get_item(1)?.is_none() {
        return Ok(None);
    }
    capture_from_tuple(py, tuple, limits)
}

/// Capture `(type, value, traceback)` from an `exc_info` triple.
fn capture_from_tuple(
    py: Python<'_>,
    tuple: &Bound<'_, PyTuple>,
    limits: &CaptureLimits,
) -> PyResult<Option<ExceptionPayload>> {
    let exc_value = tuple.get_item(1)?;
    if exc_value.is_none() {
        return Ok(None);
    }
    let exc_tb = tuple.get_item(2)?;
    let traceback = (!exc_tb.is_none()).then_some(exc_tb);

    Ok(Some(capture_with_fallback(
        py,
        &exc_value,
        Traceback::Explicit(traceback),
        limits,
    )))
}

/// Decide how `exc_info` was supplied, rejecting anything unrecognized.
fn classify_exc_info(py: Python<'_>, exc_info: &Bound<'_, PyAny>) -> PyResult<ExcInfoKind> {
    if let Ok(flag) = exc_info.cast::<PyBool>() {
        return Ok(if flag.is_true() {
            ExcInfoKind::Current
        } else {
            ExcInfoKind::Nothing
        });
    }
    if exc_info.is_none() {
        return Ok(ExcInfoKind::Nothing);
    }
    if let Ok(tuple) = exc_info.cast::<PyTuple>()
        && tuple.len() == 3
    {
        types::validate_tuple_members(py, tuple)?;
        return Ok(ExcInfoKind::Tuple(tuple.clone().unbind()));
    }
    if is_exception_instance(py, exc_info)? {
        return Ok(ExcInfoKind::Instance);
    }
    Err(pyo3::exceptions::PyTypeError::new_err(
        "exc_info must be True, an exception instance, or a 3-tuple (type, value, traceback)",
    ))
}

/// Check whether `obj` is an exception instance.
///
/// The check uses the object's real type rather than `isinstance`, which
/// consults a `__class__` property that an ordinary object can fake.
fn is_exception_instance(py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<bool> {
    Ok(types::is_exception(py, obj))
}

#[cfg(test)]
mod counter {
    //! Test-only capture counter.
    //!
    //! The decisive property of the deferred-capture design is that some records
    //! are logged *without* any traversal running. Asserting that needs a count
    //! of traversals, which the payloads themselves cannot show: an uncaptured
    //! record and a record whose exception had nothing to record look identical.

    use std::cell::Cell;

    thread_local! {
        /// Traversals performed by the current thread.
        static CAPTURES: Cell<usize> = const { Cell::new(0) };
    }

    /// Record one traversal.
    pub(crate) fn record_capture() {
        CAPTURES.with(|count| count.set(count.get() + 1));
    }

    /// Return how many traversals the current thread has performed.
    pub(crate) fn capture_count() -> usize {
        CAPTURES.with(Cell::get)
    }

    /// Reset the current thread's traversal count to zero.
    pub(crate) fn reset_capture_count() {
        CAPTURES.with(|count| count.set(0));
    }
}
