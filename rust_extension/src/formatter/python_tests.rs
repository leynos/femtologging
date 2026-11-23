//! Unit tests for the Python formatter's payload conversion.
//!
//! `record_to_dict` and `exception_payload_to_py` in the parent `python` module
//! hand a Python callable a mapping. Python dicts preserve insertion order and
//! a caller may iterate them, so the key order is part of the payload contract
//! rather than an implementation detail. These tests pin that order after the
//! exception conversion was split into helpers, so an extraction that reorders
//! the writes fails here instead of reaching a caller. The record-conversion
//! helpers are covered beside it, including the pre-epoch timestamp floor.

use std::collections::BTreeMap;
use std::time::{Duration, UNIX_EPOCH};

use pyo3::{prelude::*, types::PyDict};

use super::{exception_payload_to_py, key_values_to_dict, unix_seconds};
use crate::exception_schema::{ExceptionPayload, StackFrame};
use crate::log_record::RecordMetadata;

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

#[test]
fn unix_seconds_treats_pre_epoch_times_as_zero() {
    let metadata = RecordMetadata {
        timestamp: UNIX_EPOCH - Duration::from_secs(1),
        ..RecordMetadata::default()
    };
    assert_eq!(unix_seconds(&metadata), 0.0);
}

#[test]
fn key_values_to_dict_copies_structured_values() {
    Python::attach(|py| {
        let metadata = RecordMetadata {
            key_values: BTreeMap::from([("request_id".to_owned(), "abc".to_owned())]),
            ..RecordMetadata::default()
        };
        let dict = key_values_to_dict(py, &metadata.key_values).expect("build key-value dict");
        let request_id: String = dict
            .get_item("request_id")
            .expect("lookup")
            .expect("present")
            .extract()
            .expect("string value");
        assert_eq!(request_id, "abc");
    });
}
