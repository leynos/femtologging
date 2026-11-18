//! Integration tests for the formatter's rendering of Python-facing log records.

use _femtologging_rs::{DefaultFormatter, FemtoFormatter, FemtoLevel, FemtoLogRecord};
use rstest::rstest;

#[rstest]
#[case("core", FemtoLevel::Info, "hello", "core [INFO] hello")]
#[case("sys", FemtoLevel::Error, "fail", "sys [ERROR] fail")]
#[case("", FemtoLevel::Info, "", " [INFO] ")]
#[case("core", FemtoLevel::Warn, "⚠", "core [WARN] ⚠")]
#[case("core", FemtoLevel::Info, "one\ntwo", "core [INFO] one\\ntwo")]
#[case("core", FemtoLevel::Info, "one\rtwo", "core [INFO] one\\rtwo")]
#[case("core", FemtoLevel::Info, "one\r\ntwo", "core [INFO] one\\r\\ntwo")]
#[case("core", FemtoLevel::Info, "one\u{000B}two", "core [INFO] one\\u{b}two")]
#[case("core", FemtoLevel::Info, "one\u{000C}two", "core [INFO] one\\u{c}two")]
#[case(
    "core",
    FemtoLevel::Info,
    "one\u{001C}two",
    "core [INFO] one\\u{1c}two"
)]
#[case(
    "core",
    FemtoLevel::Info,
    "one\u{001D}two",
    "core [INFO] one\\u{1d}two"
)]
#[case(
    "core",
    FemtoLevel::Info,
    "one\u{001E}two",
    "core [INFO] one\\u{1e}two"
)]
#[case(
    "core",
    FemtoLevel::Info,
    "one\u{0085}two",
    "core [INFO] one\\u{85}two"
)]
#[case(
    "core",
    FemtoLevel::Info,
    "one\u{2028}two",
    "core [INFO] one\\u{2028}two"
)]
#[case(
    "core",
    FemtoLevel::Info,
    "one\u{2029}two",
    "core [INFO] one\\u{2029}two"
)]
#[case("core", FemtoLevel::Info, "trailing\n", "core [INFO] trailing\\n")]
#[case(
    "core",
    FemtoLevel::Info,
    "forged\nother [CRITICAL] message",
    "core [INFO] forged\\nother [CRITICAL] message"
)]
#[case(
    "core",
    FemtoLevel::Info,
    "forged\rother [CRITICAL] message",
    "core [INFO] forged\\rother [CRITICAL] message"
)]
#[case(
    "core",
    FemtoLevel::Info,
    "forged\r\nother [CRITICAL] message",
    "core [INFO] forged\\r\\nother [CRITICAL] message"
)]
#[case(
    "core",
    FemtoLevel::Info,
    r"literal\nsequence",
    r"core [INFO] literal\nsequence"
)]
#[case("core", FemtoLevel::Info, "column\tvalue", "core [INFO] column\tvalue")]
fn default_formatter_formats(
    #[case] logger: &str,
    #[case] level: FemtoLevel,
    #[case] message: &str,
    #[case] expected: &str,
) {
    let record = FemtoLogRecord::new(logger, level, message);
    let formatter = DefaultFormatter;
    assert_eq!(formatter.format(&record), expected);
}
