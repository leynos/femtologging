//! Verify required frame fields report their missing key by name.

use pyo3::prelude::*;
use pyo3::types::PyDict;

use super::require_frame_item;

#[test]
fn missing_key_names_the_field() {
    Python::attach(|py| {
        let dict = PyDict::new(py);
        dict.set_item("filename", "module.py")
            .expect("set filename");
        let filename: String = require_frame_item(&dict, "filename")
            .expect("filename present")
            .extract()
            .expect("filename extracts");
        assert_eq!(filename, "module.py");

        let error = require_frame_item(&dict, "lineno").expect_err("lineno is absent");
        assert!(
            error
                .to_string()
                .contains("frame dict missing 'lineno' key"),
            "unexpected error: {error}"
        );
    });
}
