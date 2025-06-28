use _femtologging_rs::FemtoLogger;
use rstest::rstest;

#[rstest]
#[case("core", "INFO", "hello", "core [INFO] hello")]
#[case("sys", "ERROR", "fail", "sys [ERROR] fail")]
#[case("", "INFO", "", " [INFO] ")]
#[case("core", "WARN", "⚠", "core [WARN] ⚠")]
#[case("i18n", "INFO", "こんにちは世界", "i18n [INFO] こんにちは世界")]
fn log_formats_message(
    #[case] name: &str,
    #[case] level: &str,
    #[case] message: &str,
    #[case] expected: &str,
) {
    let logger = FemtoLogger::new(name.to_string());
    assert_eq!(logger.log(level, message), expected);
}

#[test]
fn log_formats_very_long_message() {
    let long_msg = "x".repeat(1024);
    let logger = FemtoLogger::new("long".to_string());
    let expected = format!("long [INFO] {}", long_msg);
    assert_eq!(logger.log("INFO", &long_msg), expected);
}
