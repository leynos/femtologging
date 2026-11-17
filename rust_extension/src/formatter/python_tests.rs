//! Tests for record conversion helpers.

use std::collections::BTreeMap;
use std::time::{Duration, UNIX_EPOCH};

use pyo3::Python;
use pyo3::types::{PyAnyMethods, PyDictMethods};

use super::{key_values_to_py, timestamp_seconds};
use crate::log_record::RecordMetadata;

#[test]
fn timestamp_seconds_treats_pre_epoch_times_as_zero() {
    let metadata = RecordMetadata {
        timestamp: UNIX_EPOCH - Duration::from_secs(1),
        ..RecordMetadata::default()
    };
    assert_eq!(timestamp_seconds(&metadata), 0.0);
}

#[test]
fn key_values_to_py_copies_structured_values() {
    Python::attach(|py| {
        let metadata = RecordMetadata {
            key_values: BTreeMap::from([("request_id".to_owned(), "abc".to_owned())]),
            ..RecordMetadata::default()
        };
        let dict = key_values_to_py(py, &metadata).expect("build key-value dict");
        let request_id: String = dict
            .get_item("request_id")
            .expect("lookup")
            .expect("present")
            .extract()
            .expect("string value");
        assert_eq!(request_id, "abc");
    });
}
