//! Verify required frame fields report their missing key by name.

use pyo3::prelude::*;
use pyo3::types::PyDict;

use super::required_field;

#[test]
fn missing_key_names_the_field() {
    Python::attach(|py| {
        let dict = PyDict::new(py);
        dict.set_item("filename", "module.py")
            .expect("set filename");
        let filename: String = required_field(&dict, "filename").expect("filename present");
        assert_eq!(filename, "module.py");

        let error = required_field::<u32>(&dict, "lineno").expect_err("lineno is absent");
        assert!(
            error
                .to_string()
                .contains("frame dict missing 'lineno' key"),
            "unexpected error: {error}"
        );
    });
}
