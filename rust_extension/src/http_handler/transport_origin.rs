//! Track whether the current thread is running HTTP transport work.
//!
//! An HTTP handler delivers records on a dedicated worker thread. That worker
//! uses `ureq`, which reports its own progress through the `log` and `tracing`
//! facades. When the process-wide bridges are installed, those transport
//! diagnostics are ordinary records: a diagnostic target such as `ureq.run`
//! can route them back to the same HTTP handler that produced them, and each
//! successful delivery then manufactures more records to deliver.
//!
//! The marker is scoped to the OS thread because a worker owns the whole
//! delivery lifetime. The bridges capture it once, at entry, and suspend it
//! while they run user-visible work, so a filter, formatter, or handler
//! callback that logs during dispatch produces an ordinary application record
//! rather than an inherited transport one.
//!
//! The marker is thread-local rather than process-wide: two HTTP handlers on
//! different threads must not hide each other's ordinary records, and a
//! record emitted from an application thread stays deliverable even while
//! some other thread is inside a request.

use std::cell::Cell;
use std::marker::PhantomData;
use std::rc::Rc;

#[cfg(any(feature = "log-compat", feature = "tracing-compat", test))]
use crate::log_record::RecordOrigin;

thread_local! {
    /// Whether the current thread is inside HTTP transport work.
    static HTTP_TRANSPORT_ACTIVE: Cell<bool> = const { Cell::new(false) };
}

// The bridge-only API is absent when neither bridge is compiled, so a plain
// `--no-default-features` library build has no dead code under `-D warnings`.
/// Return whether the current thread is inside HTTP transport work.
#[cfg(any(feature = "log-compat", feature = "tracing-compat", test))]
pub(crate) fn is_http_transport_active() -> bool {
    HTTP_TRANSPORT_ACTIVE.with(Cell::get)
}

/// Capture the origin a bridge entry should attach to its record.
///
/// The bridge captures this before suspending the scope, so a record produced
/// by transport code keeps the marker while records produced by anything the
/// bridge calls do not.
#[cfg(any(feature = "log-compat", feature = "tracing-compat", test))]
pub(crate) fn capture_record_origin() -> RecordOrigin {
    if is_http_transport_active() {
        RecordOrigin::SelfDiagnostic
    } else {
        RecordOrigin::Application
    }
}

/// A scope whose drop restores the thread's previous transport state.
///
/// The guard is deliberately not `Send` or `Sync`: the state is OS-thread
/// local, so a guard must stay on the thread that created it. Restoration is
/// unconditional, which also covers a panic unwinding through a suspended
/// bridge call.
#[must_use = "hold the guard for as long as the transport state should remain in effect"]
pub(crate) struct TransportScope {
    /// State restored when this scope ends.
    previous: bool,
    /// Marker preventing the guard from crossing an OS-thread boundary.
    _not_send_or_sync: PhantomData<Rc<()>>,
}

impl Drop for TransportScope {
    fn drop(&mut self) {
        HTTP_TRANSPORT_ACTIVE.with(|active| active.set(self.previous));
    }
}

/// Mark the rest of the current thread's work as HTTP transport.
pub(crate) fn enter_http_transport() -> TransportScope {
    set_transport_state(true)
}

/// Suspend transport marking for nested work on the current thread.
///
/// For example, a bridge suspends the scope after capturing the origin so a
/// Python formatter or handler invoked later in the same call does not emit
/// records that inherit the transport marker.
#[cfg(any(feature = "log-compat", feature = "tracing-compat", test))]
pub(crate) fn suspend_http_transport() -> TransportScope {
    set_transport_state(false)
}

/// Apply `next` and remember the state a drop must restore.
fn set_transport_state(next: bool) -> TransportScope {
    let previous = HTTP_TRANSPORT_ACTIVE.with(|active| active.replace(next));
    TransportScope {
        previous,
        _not_send_or_sync: PhantomData,
    }
}

#[cfg(test)]
#[path = "transport_origin_tests.rs"]
mod tests;
