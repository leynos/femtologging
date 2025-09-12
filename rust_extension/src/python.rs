#![cfg(feature = "python")]

//! Shared helpers for Python interaction.
//!
//! Utilities to keep Python-only code deduplicated across modules.

use pyo3::prelude::*;

/// Return the fully qualified Python type name for `obj`.
///
/// Falls back to `"<unknown>"` when the module or qualified name cannot
/// be retrieved.
pub(crate) fn fq_py_type(obj: &Bound<'_, PyAny>) -> String {
    let ty = obj.get_type();
    let module = ty
        .getattr("__module__")
        .and_then(|m| m.extract::<String>())
        .unwrap_or_else(|_| "<unknown>".to_string());
    let qualname = ty
        .getattr("__qualname__")
        .and_then(|n| n.extract::<String>())
        .unwrap_or_else(|_| "<unknown>".to_string());
    if module == "builtins" {
        qualname
    } else {
        format!("{module}.{qualname}")
    }
}
