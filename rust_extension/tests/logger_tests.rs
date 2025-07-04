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
    assert_eq!(logger.log(level, message).as_deref(), Some(expected));
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
    assert_eq!(logger.log("INFO", &msg).as_deref(), Some(expected.as_str()));
}

#[test]
fn logger_filters_levels() {
    let logger = FemtoLogger::new("core".to_string());
    logger.set_level("ERROR");
    assert_eq!(logger.log("INFO", "ignored"), None);
    assert_eq!(
        logger.log("ERROR", "processed").as_deref(),
        Some("core [ERROR] processed")
    );
}

#[test]
fn level_parsing_and_filtering() {
    let logger = FemtoLogger::new("core".to_string());
    for lvl in ["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "CRITICAL"] {
        logger.set_level(lvl);
        assert!(logger.log(lvl, "ok").is_some());
    }

    logger.set_level("ERROR");
    assert!(logger.log("WARN", "drop").is_none());
    // Invalid strings default to INFO with warning
    assert!(logger.log("bogus", "drop").is_none());
}
