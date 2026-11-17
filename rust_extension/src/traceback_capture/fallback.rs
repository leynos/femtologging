//! Type-only payloads used when exception traversal cannot continue.
//!
//! If the bounded walker panics or its node allowance is zero, logging still
//! needs a small payload that identifies the exception. This module keeps that
//! fallback inside the same aggregate byte ceiling as ordinary capture.

use pyo3::prelude::*;

use crate::exception_schema::{EXCEPTION_SCHEMA_VERSION, ExceptionPayload, PayloadTruncation};

use super::attrs::type_identity;
use super::limits::{CaptureBudget, CaptureLimits};

/// Build a type-only payload under the aggregate byte budget.
pub(super) fn minimal_payload(
    exception: &Bound<'_, PyAny>,
    limits: &CaptureLimits,
) -> ExceptionPayload {
    let mut budget = CaptureBudget::new(limits);
    let (type_name, module) = type_identity(exception, limits);
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
