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

#[rstest]
#[case(0)]
#[case(1024)]
#[case(65536)]
#[case(1_048_576)]
fn log_formats_long_messages(#[case] length: usize) {
    let msg = "x".repeat(length);
    let logger = FemtoLogger::new("long".to_string());
    let expected = format!("long [INFO] {}", msg);
    assert_eq!(logger.log("INFO", &msg), expected);
}

#[test]
fn logger_skips_disabled_levels() {
    let logger = FemtoLogger::new("core".to_string());
    assert_eq!(logger.log("DEBUG", "hidden"), "");
}

#[test]
fn logger_respects_set_level() {
    let mut logger = FemtoLogger::new("core".to_string());
    logger.set_level("DEBUG").unwrap();
    assert_eq!(logger.log("DEBUG", "shown"), "core [DEBUG] shown");
}

#[test]
fn logger_invalid_level_defaults_to_info() {
    let logger = FemtoLogger::new("core".to_string());
    assert_eq!(logger.log("BAD", "msg"), "core [INFO] msg");
}
