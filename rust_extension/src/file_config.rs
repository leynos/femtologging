#![cfg(feature = "python")]

//! INI parsing helpers exposed to Python.
//!
//! The Python `fileConfig` implementation delegates INI parsing to Rust so we
//! can reuse the `rust-ini` crate and keep the parser consistent across
//! platforms.

use encoding_rs::Encoding;
use ini::Ini;
use pyo3::exceptions::{
    PyFileNotFoundError, PyIOError, PyLookupError, PyRuntimeError, PyUnicodeDecodeError,
};
use pyo3::prelude::*;
use std::fs;
use std::io::ErrorKind;

type SectionEntries = Vec<(String, String)>;
type ParsedSections = Vec<(String, SectionEntries)>;

#[pyfunction]
pub(crate) fn parse_ini_file(
    py: Python<'_>,
    path: &str,
    encoding: Option<&str>,
) -> PyResult<ParsedSections> {
    let bytes = read_file_bytes(path)?;
    if bytes.is_empty() {
        return Err(PyRuntimeError::new_err(format!("{path} is an empty file")));
    }
    let text = decode_contents(py, &bytes, encoding)?;
    parse_sections(path, &text)
}

fn read_file_bytes(path: &str) -> PyResult<Vec<u8>> {
    match fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(err) => match err.kind() {
            ErrorKind::NotFound => Err(PyFileNotFoundError::new_err(format!(
                "{path} doesn't exist"
            ))),
            _ => Err(PyIOError::new_err(format!("failed to read {path}: {err}"))),
        },
    }
}

fn decode_contents<'a>(py: Python<'a>, bytes: &[u8], encoding: Option<&str>) -> PyResult<String> {
    match encoding {
        Some(label) => decode_with_encoding(py, bytes, label),
        None => decode_utf8(py, bytes),
    }
}

fn decode_utf8(py: Python<'_>, bytes: &[u8]) -> PyResult<String> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text.to_owned()),
        Err(err) => {
            let start = err.valid_up_to();
            let end = start + err.error_len().unwrap_or(1);
            Err(unicode_decode_err(
                py,
                UnicodeDecodeErrorInfo {
                    encoding: "utf-8",
                    bytes,
                    start,
                    end,
                    reason: "invalid utf-8 sequence",
                },
            ))
        }
    }
}

fn decode_with_encoding(py: Python<'_>, bytes: &[u8], label: &str) -> PyResult<String> {
    let normalized_label = label.trim().to_ascii_lowercase();
    let encoding = Encoding::for_label(normalized_label.as_bytes())
        .ok_or_else(|| PyLookupError::new_err(format!("unknown encoding {label}")))?;
    let (decoded, _, had_errors) = encoding.decode(bytes);
    if had_errors {
        return Err(unicode_decode_err(
            py,
            UnicodeDecodeErrorInfo {
                encoding: encoding.name(),
                bytes,
                start: 0,
                end: bytes.len(),
                reason: "decoding error",
            },
        ));
    }
    Ok(decoded.into_owned())
}

struct UnicodeDecodeErrorInfo<'a> {
    encoding: &'a str,
    bytes: &'a [u8],
    start: usize,
    end: usize,
    reason: &'a str,
}

fn unicode_decode_err(_py: Python<'_>, info: UnicodeDecodeErrorInfo<'_>) -> PyErr {
    PyUnicodeDecodeError::new_err((
        info.encoding.to_string(),
        info.bytes.to_vec(),
        info.start,
        info.end.min(info.bytes.len()),
        info.reason.to_string(),
    ))
}

fn parse_sections(path: &str, text: &str) -> PyResult<ParsedSections> {
    let ini = Ini::load_from_str(text)
        .map_err(|err| PyRuntimeError::new_err(format!("{path} is invalid: {err}")))?;
    Ok(ini
        .iter()
        .map(|(section, props)| {
            let name = section
                .map(|s| s.to_string())
                .unwrap_or_else(|| "DEFAULT".to_string());
            let entries = props
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_owned()))
                .collect();
            (name, entries)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::{decode_contents, parse_ini_file, parse_sections};
    use pyo3::exceptions::PyLookupError;
    use pyo3::Python;
    use rstest::rstest;
    use tempfile::NamedTempFile;

    #[rstest]
    fn parses_sections_in_order() {
        let contents = r#"[DEFAULT]
key = value

[loggers]
keys = root

[logger_root]
level = INFO
"#;
        let parsed = parse_sections("config.ini", contents).unwrap();
        let names: Vec<_> = parsed.iter().map(|(name, _)| name).collect();
        assert_eq!(names, &["DEFAULT", "loggers", "logger_root"]);
        assert_eq!(parsed[0].1[0], ("key".to_string(), "value".to_string()));
    }

    #[rstest]
    fn decode_rejects_unknown_encoding() {
        Python::with_gil(|py| {
            let err = decode_contents(py, b"data", Some("does-not-exist"))
                .expect_err("expected lookup failure");
            assert!(err.is_instance_of::<PyLookupError>(py));
        });
    }

    #[rstest]
    fn parse_ini_file_reads_from_disk() {
        let mut file = NamedTempFile::new().unwrap();
        use std::io::Write;
        writeln!(
            file,
            "[loggers]\nkeys = root\n\n[logger_root]\nlevel = INFO\nhandlers = console"
        )
        .unwrap();
        let path = file.path().display().to_string();
        Python::with_gil(|py| {
            let sections = parse_ini_file(py, &path, None).expect("should parse");
            assert_eq!(sections.len(), 2);
        });
    }
}
