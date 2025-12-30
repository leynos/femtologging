use _femtologging_rs::{DefaultFormatter, FemtoFormatter, FemtoLevel, FemtoLogRecord};
use rstest::rstest;

#[rstest]
#[case("core", FemtoLevel::Info, "hello", "core [INFO] hello")]
#[case("sys", FemtoLevel::Error, "fail", "sys [ERROR] fail")]
#[case("", FemtoLevel::Info, "", " [INFO] ")]
#[case("core", FemtoLevel::Warn, "⚠", "core [WARN] ⚠")]
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
