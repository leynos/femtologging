//! Type-only payloads used when exception traversal cannot continue.
//!
//! If the bounded walker panics or its node allowance is zero, logging still
//! needs a small payload that identifies the exception. This module keeps that
//! fallback inside the same aggregate byte ceiling as ordinary capture.

use std::panic::{AssertUnwindSafe, catch_unwind};

use pyo3::prelude::*;

use crate::exception_schema::{EXCEPTION_SCHEMA_VERSION, ExceptionPayload, PayloadTruncation};

use super::attrs::type_identity;
use super::limits::{CaptureBudget, CaptureLimits};

/// Type name used when even the type lookup cannot be completed.
const TYPE_PLACEHOLDER: &str = "<exception type unavailable>";

/// Build a type-only payload under the aggregate byte budget.
///
/// Identity recovery is guarded on its own. This function is already the
/// fallback for a panicking traversal, and a metaclass that panics on
/// `__name__` would otherwise panic again here, outside any guard, and leave
/// logging with no payload at all.
pub(super) fn minimal_payload(
    exception: &Bound<'_, PyAny>,
    limits: &CaptureLimits,
) -> ExceptionPayload {
    let mut budget = CaptureBudget::new(limits);
    let (type_name, module) = catch_unwind(AssertUnwindSafe(|| type_identity(exception, limits)))
        .unwrap_or_else(|_| {
            // A panic can leave an exception indicator set; clearing it stops a
            // later Python call from reporting an unrelated failure.
            let _ = PyErr::take(exception.py());
            (TYPE_PLACEHOLDER.to_string(), None)
        });
    // Both name lookups can fail without raising: a metaclass may hide the
    // attributes. An empty identity says nothing, so substitute the placeholder
    // before the byte budget is charged and let a zero-byte limit still empty it.
    let type_name = if type_name.is_empty() {
        TYPE_PLACEHOLDER.to_string()
    } else {
        type_name
    };
    let (type_name, _) = budget.retain_text(type_name);
    let module = module.and_then(|module| {
        let (module, _) = budget.retain_text(module);
        (!module.is_empty()).then_some(module)
    });
    ExceptionPayload {
        schema_version: EXCEPTION_SCHEMA_VERSION,
        type_name,
        module,
        truncation: Some(PayloadTruncation {
            budget_exhausted: true,
            ..Default::default()
        }),
        ..Default::default()
    }
}
