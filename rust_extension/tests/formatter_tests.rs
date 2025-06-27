use _femtologging_rs::{DefaultFormatter, FemtoFormatter, FemtoLogRecord};
use rstest::rstest;

#[rstest]
#[case("core", "INFO", "hello", "core: INFO - hello")]
#[case("sys", "ERROR", "fail", "sys: ERROR - fail")]
#[case("", "INFO", "", ": INFO - ")]
#[case("core", "WARN", "⚠", "core: WARN - ⚠")]
fn default_formatter_formats(
    #[case] logger: &str,
    #[case] level: &str,
    #[case] message: &str,
    #[case] expected: &str,
) {
    let record = FemtoLogRecord::new(logger, level, message);
    let formatter = DefaultFormatter;
    assert_eq!(formatter.format(&record), expected);
}
