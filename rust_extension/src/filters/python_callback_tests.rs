//! Tests for Python callback filter validation and enrichment behaviour.

use std::collections::BTreeMap;
use std::ffi::CString;

use pyo3::prelude::*;
use pyo3::types::PyModule;
use rstest::rstest;

use super::python_callback::{PythonCallbackFilter, PythonCallbackFilterBuilder};
use super::python_callback_validation::{
    extract_supported_value, validate_enrichment_key, validate_enrichment_total,
    validate_enrichment_value,
};
use crate::filters::{FemtoFilter, FilterBuilderTrait};
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;

#[rstest]
#[case("levelname")]
#[case("message")]
#[case("metadata")]
fn reserved_keys_are_rejected(#[case] key: &str) {
    assert!(validate_enrichment_key(key).is_err());
}

#[rstest]
#[case("k".repeat(64), true)]
#[case("k".repeat(65), false)]
fn key_length_is_bounded(#[case] key: String, #[case] expected_ok: bool) {
    assert_eq!(validate_enrichment_key(&key).is_ok(), expected_ok);
}

#[rstest]
#[case("v".repeat(1024), true)]
#[case("v".repeat(1025), false)]
fn value_length_is_bounded(#[case] value: String, #[case] expected_ok: bool) {
    assert_eq!(
        validate_enrichment_value("field", &value).is_ok(),
        expected_ok
    );
}

#[rstest]
#[case(64, true)]
#[case(65, false)]
fn total_key_count_is_bounded(#[case] key_count: usize, #[case] expected_ok: bool) {
    let enrichment = (0..key_count)
        .map(|index| (format!("k{index}"), "v".to_owned()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(validate_enrichment_total(&enrichment).is_ok(), expected_ok);
}

#[test]
fn total_payload_size_is_bounded() {
    let within_limit = BTreeMap::from([("a".repeat(64), "b".repeat(1024))]);
    assert!(validate_enrichment_total(&within_limit).is_ok());

    let mut over_limit = BTreeMap::new();
    for index in 0..17 {
        over_limit.insert(format!("field_{index}"), "v".repeat(1024));
    }
    assert!(validate_enrichment_total(&over_limit).is_err());
}

#[test]
fn supported_python_scalars_are_stringified() {
    Python::attach(|py| {
        let values = [
            ("'hello'", "hello"),
            ("42", "42"),
            ("3.5", "3.5"),
            ("True", "True"),
            ("None", "None"),
        ];

        for (expr, expected) in values {
            let code = CString::new(expr).expect("valid Python expression");
            let value = py
                .eval(code.as_c_str(), None, None)
                .expect("expression should evaluate");
            let actual =
                extract_supported_value("field", &value).expect("value should be accepted");
            assert_eq!(actual, expected);
        }
    });
}

#[test]
fn unsupported_python_values_are_rejected() {
    Python::attach(|py| {
        let module = PyModule::from_code(
            py,
            CString::new("class Custom: pass\nvalue = Custom()\n")
                .expect("valid Python code")
                .as_c_str(),
            CString::new("test_callback_filter.py")
                .expect("valid filename")
                .as_c_str(),
            CString::new("test_callback_filter")
                .expect("valid module name")
                .as_c_str(),
        )
        .expect("module creation should succeed");
        let value = module.getattr("value").expect("value should exist");
        assert!(extract_supported_value("field", &value).is_err());
    });
}

#[test]
fn python_callback_filter_extracts_enrichment() {
    Python::attach(|py| {
        let module = PyModule::from_code(
            py,
            CString::new(concat!(
                "def enrich(record):\n",
                "    record.request_id = 'abc-123'\n",
                "    return True\n",
            ))
            .expect("valid Python code")
            .as_c_str(),
            CString::new("test_enrich.py")
                .expect("valid filename")
                .as_c_str(),
            CString::new("test_enrich")
                .expect("valid module name")
                .as_c_str(),
        )
        .expect("module creation should succeed");
        let builder = PythonCallbackFilterBuilder::from_callback_obj(
            module.getattr("enrich").expect("function should exist"),
        )
        .expect("builder should be created");
        let filter = builder.build_inner().expect("filter should build");
        let record = FemtoLogRecord::new("core", FemtoLevel::Info, "hello");

        let decision = filter
            .filter_with_enrichment(&record)
            .expect("filter should run");

        assert!(decision.accepted);
        assert_eq!(
            decision.enrichment.get("request_id"),
            Some(&"abc-123".to_owned())
        );
    });
}

#[test]
fn python_callback_filter_supports_filter_method_objects() {
    Python::attach(|py| {
        let module = PyModule::from_code(
            py,
            CString::new(concat!(
                "class OnlyInfo:\n",
                "    def filter(self, record):\n",
                "        return record.levelname == 'INFO'\n",
            ))
            .expect("valid Python code")
            .as_c_str(),
            CString::new("test_filter_obj.py")
                .expect("valid filename")
                .as_c_str(),
            CString::new("test_filter_obj")
                .expect("valid module name")
                .as_c_str(),
        )
        .expect("module creation should succeed");
        let instance = module
            .getattr("OnlyInfo")
            .expect("class should exist")
            .call0()
            .expect("instance construction should succeed");
        let filter =
            PythonCallbackFilter::new(instance.unbind(), "test_filter_obj.OnlyInfo".into());

        assert!(filter.should_log(&FemtoLogRecord::new("core", FemtoLevel::Info, "hello",)));
        assert!(!filter.should_log(&FemtoLogRecord::new("core", FemtoLevel::Error, "hello",)));
    });
}
