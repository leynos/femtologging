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

/// Entries retained from one INI section, preserving the parser's key order.
type SectionEntries = Vec<(String, String)>;
/// Parsed INI sections paired with their entries, including the synthetic `DEFAULT` name.
type ParsedSections = Vec<(String, SectionEntries)>;

/// Read, decode, and parse a Python `fileConfig` INI file without changing logger state.
///
/// Missing files, unknown encodings, decoding failures, empty files, and invalid
/// INI syntax are reported as Python exceptions before the caller can apply the
/// returned sections.
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

/// Read the raw bytes of `path`, mapping filesystem failures to Python I/O errors.
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

/// Decode file bytes using the explicit encoding or Python's preferred encoding.
fn decode_contents<'a>(py: Python<'a>, bytes: &[u8], encoding: Option<&str>) -> PyResult<String> {
    match encoding {
        Some(label) => decode_with_encoding(py, bytes, label),
        None => {
            let label = preferred_encoding(py)?;
            decode_with_encoding(py, bytes, &label)
        }
    }
}

/// Ask Python for the locale-preferred encoding used when `fileConfig` omits one.
fn preferred_encoding(py: Python<'_>) -> PyResult<String> {
    let locale = py.import("locale")?;
    let func = locale.getattr("getpreferredencoding")?;
    func.call1((false,))?.extract::<String>()
}

/// Decodes UTF-8 directly so tests can exercise the same Python error shape as configured decoding.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Used only in tests; production path calls decode_with_encoding"
    )
)]
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

/// Decode bytes with a named `encoding_rs` codec and preserve Unicode error details.
fn decode_with_encoding(py: Python<'_>, bytes: &[u8], label: &str) -> PyResult<String> {
    let normalized_label = label.trim().to_ascii_lowercase();
    let encoding = Encoding::for_label(normalized_label.as_bytes())
        .ok_or_else(|| PyLookupError::new_err(format!("unknown encoding {label}")))?;
    let (decoded, _, had_errors) = encoding.decode(bytes);
    if had_errors {
        // encoding_rs does not surface precise error offsets, so report the
        // entire byte range to keep Python's UnicodeDecodeError consistent.
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

/// Describe the byte range and codec needed to construct a Python decode error.
struct UnicodeDecodeErrorInfo<'a> {
    /// Codec name reported by Python in the exception.
    encoding: &'a str,
    /// Original bytes retained for Python's `UnicodeDecodeError` payload.
    bytes: &'a [u8],
    /// First byte offset of the reported decoding failure.
    start: usize,
    /// Exclusive end offset, clamped when converted into the Python exception.
    end: usize,
    /// Human-readable reason attached to the decode failure.
    reason: &'a str,
}

/// Convert decoding details into the Python `UnicodeDecodeError` shape.
fn unicode_decode_err(_py: Python<'_>, info: UnicodeDecodeErrorInfo<'_>) -> PyErr {
    PyUnicodeDecodeError::new_err((
        info.encoding.to_string(),
        info.bytes.to_vec(),
        info.start,
        info.end.min(info.bytes.len()),
        info.reason.to_string(),
    ))
}

/// Parse INI sections into owned entries while retaining section order.
///
/// The nameless section is exposed as `DEFAULT`; an empty nameless section is
/// omitted because it carries no configuration.
fn parse_sections(path: &str, text: &str) -> PyResult<ParsedSections> {
    let ini = Ini::load_from_str(text)
        .map_err(|err| PyRuntimeError::new_err(format!("{path} is invalid: {err}")))?;
    Ok(ini
        .iter()
        .filter_map(|(section, props)| {
            let entries: Vec<(String, String)> = props
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_owned()))
                .collect();
            if section.is_none() && entries.is_empty() {
                return None;
            }
            let name = section
                .map(|s| s.to_string())
                .unwrap_or_else(|| "DEFAULT".to_string());
            Some((name, entries))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    //! Tests for fileConfig parsing and decoding.

    use super::{decode_contents, decode_utf8, parse_ini_file, parse_sections};

    use pyo3::exceptions::{PyLookupError, PyRuntimeError, PyUnicodeDecodeError};
    use pyo3::prelude::*;
    use pyo3::types::PyAnyMethods;
    use rstest::rstest;
    use std::ffi::CString;
    use std::sync::Mutex;
    use tempfile::NamedTempFile;

    static LOCALE_MUTATION_GUARD: Mutex<()> = Mutex::new(());

    #[rstest]
    fn parses_sections_in_order() {
        let contents = r#"[DEFAULT]
key = value

[loggers]
keys = root

[logger_root]
level = INFO
"#;
        let parsed = parse_sections("config.ini", contents).expect("parse_sections succeeds");
        let names: Vec<_> = parsed.iter().map(|(name, _)| name).collect();
        assert_eq!(names, &["DEFAULT", "loggers", "logger_root"]);
        assert_eq!(parsed[0].1[0], ("key".to_string(), "value".to_string()));
    }

    #[rstest]
    fn decode_rejects_unknown_encoding() {
        Python::attach(|py| {
            let err = decode_contents(py, b"data", Some("does-not-exist"))
                .expect_err("expected lookup failure");
            assert!(err.is_instance_of::<PyLookupError>(py));
        });
    }

    #[rstest]
    fn parse_ini_file_reads_from_disk() {
        let mut file = NamedTempFile::new().expect("create temp ini file");
        use std::io::Write;
        writeln!(
            file,
            "[loggers]\nkeys = root\n\n[logger_root]\nlevel = INFO\nhandlers = console"
        )
        .expect("write ini contents");
        let path = file.path().display().to_string();
        Python::attach(|py| {
            let sections = parse_ini_file(py, &path, None).expect("should parse");
            assert_eq!(sections.len(), 2);
        });
    }

    #[rstest]
    fn parse_ini_file_rejects_empty_file() {
        let file = NamedTempFile::new().expect("create temp ini file");
        let path = file.path().display().to_string();
        Python::attach(|py| {
            let err = parse_ini_file(py, &path, None).expect_err("empty files must fail");
            assert!(err.is_instance_of::<PyRuntimeError>(py));
        });
    }

    #[rstest]
    fn parse_ini_file_respects_locale_default() {
        let _guard = LOCALE_MUTATION_GUARD
            .lock()
            .expect("acquire locale mutation guard");
        use std::io::Write;
        let mut file = NamedTempFile::new().expect("create temp ini file");
        file.write_all(
            b"[loggers]\nkeys = root\n\n[logger_root]\nlevel = INF\xC9\nhandlers = console",
        )
        .expect("write cp1252 ini contents");
        let path = file.path().display().to_string();
        Python::attach(|py| {
            let locale = py.import("locale").expect("import locale module");
            let original = locale
                .getattr("getpreferredencoding")
                .expect("get locale.getpreferredencoding")
                .unbind();
            let expr =
                CString::new("lambda _=False: 'cp1252'").expect("create CString for locale shim");
            let shim = py
                .eval(expr.as_c_str(), None, None)
                .expect("evaluate locale shim");
            locale
                .setattr("getpreferredencoding", shim)
                .expect("patch locale encoding");

            let result = parse_ini_file(py, &path, None);

            locale
                .setattr("getpreferredencoding", original)
                .expect("restore locale encoding");

            let sections = result.expect("cp1252 default should parse");
            assert_eq!(sections.len(), 2);
        });
    }

    #[rstest]
    fn decode_contents_handles_latin1() {
        Python::attach(|py| {
            let bytes = b"caf\xE9";
            let decoded = decode_contents(py, bytes, Some("latin1")).expect("latin1 decode");
            assert_eq!(decoded, "caf\u{e9}");
        });
    }

    #[rstest]
    fn decode_utf8_reports_error_span() {
        Python::attach(|py| {
            let bytes = b"Hello\xFF\xFE";
            let err = decode_utf8(py, bytes).expect_err("invalid utf-8 must fail");
            let value = err
                .value(py)
                .cast::<PyUnicodeDecodeError>()
                .expect("unicode decode error");
            let start: isize = value
                .getattr("start")
                .expect("unicode decode error start")
                .extract()
                .expect("extract start");
            let end: isize = value
                .getattr("end")
                .expect("unicode decode error end")
                .extract()
                .expect("extract end");
            assert_eq!((start, end), (5, 6));
        });
    }
}
