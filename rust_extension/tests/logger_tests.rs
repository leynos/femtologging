use _femtologging_rs::FemtoLogger;
use rstest::rstest;

#[rstest]
#[case("core", "INFO", "hello", "core: INFO - hello")]
#[case("sys", "ERROR", "fail", "sys: ERROR - fail")]
fn log_formats_message(
    #[case] name: &str,
    #[case] level: &str,
    #[case] message: &str,
    #[case] expected: &str,
) {
    let logger = FemtoLogger::new(name.to_string());
    assert_eq!(logger.log(level, message), expected);
}
