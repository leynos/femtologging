//! Unit tests for the Python formatter's payload conversion.
//!
//! `record_to_dict` and `exception_payload_to_py` in the parent `python` module
//! hand a Python callable a mapping. Python dicts preserve insertion order and
//! a caller may iterate them, so the key order is part of the payload contract
//! rather than an implementation detail. These tests pin that order after the
//! exception conversion was split into helpers, so an extraction that reorders
//! the writes fails here instead of reaching a caller.

use super::exception_payload_to_py;
use crate::exception_schema::{ExceptionPayload, StackFrame};
use pyo3::{prelude::*, types::PyDict};

/// Build a payload that carries every optional field the conversion can emit.
fn full_payload() -> ExceptionPayload {
    let mut payload = ExceptionPayload::new("ExceptionGroup", "two errors");
    payload.module = Some("myapp.errors".to_owned());
    payload.args_repr = vec!["'a'".to_owned()];
    payload.notes = vec!["check the input".to_owned()];
    payload.frames.push(StackFrame::new("app.py", 7, "run"));
    payload.cause = Some(Box::new(ExceptionPayload::new("KeyError", "cause")));
    payload.context = Some(Box::new(ExceptionPayload::new("OSError", "context")));
    payload.suppress_context = true;
    payload.exceptions = vec![ExceptionPayload::new("ValueError", "first")];
    payload
}

#[test]
fn the_exception_payload_keeps_its_historical_key_order() {
    Python::attach(|py| {
        let dict = exception_payload_to_py(py, &full_payload()).expect("the payload must convert");
        let keys: Vec<String> = dict
            .bind(py)
            .cast::<PyDict>()
            .expect("the payload converts to a dict")
            .keys()
            .iter()
            .map(|key| key.extract::<String>().expect("keys are strings"))
            .collect();

        assert_eq!(
            keys,
            vec![
                "schema_version",
                "type_name",
                "message",
                "module",
                "args_repr",
                "notes",
                "frames",
                "cause",
                "context",
                "suppress_context",
                "exceptions",
            ],
            "splitting the conversion must not reorder the payload keys"
        );
    });
}