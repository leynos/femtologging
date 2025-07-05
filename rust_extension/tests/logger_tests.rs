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
#[derive(Clone)]
struct SharedBuf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.lock().unwrap().flush()
    }
}

fn read_output(buf: &std::sync::Arc<std::sync::Mutex<Vec<u8>>>) -> String {
    String::from_utf8(buf.lock().unwrap().clone()).unwrap()
}

#[test]
fn logger_dispatches_to_multiple_handlers() {
    use _femtologging_rs::{DefaultFormatter, FemtoHandlerTrait, FemtoStreamHandler};
    use std::sync::{Arc, Mutex};
    let buf1 = Arc::new(Mutex::new(Vec::new()));
    let buf2 = Arc::new(Mutex::new(Vec::new()));
    let h1 = Arc::new(FemtoStreamHandler::new(
        SharedBuf(Arc::clone(&buf1)),
        DefaultFormatter,
    ));
    let h2 = Arc::new(FemtoStreamHandler::new(
        SharedBuf(Arc::clone(&buf2)),
        DefaultFormatter,
    ));

    let mut logger = FemtoLogger::new("core".to_string());
    let h1_trait: Arc<dyn FemtoHandlerTrait> = h1.clone();
    let h2_trait: Arc<dyn FemtoHandlerTrait> = h2.clone();
    logger.add_handler(h1_trait);
    logger.add_handler(h2_trait);

    logger.log("INFO", "hello");
    drop(logger);
    drop(h1);
    drop(h2);

    assert_eq!(read_output(&buf1), "core [INFO] hello\n");
    assert_eq!(read_output(&buf2), "core [INFO] hello\n");
}

#[test]
fn shared_handler_between_loggers() {
    use _femtologging_rs::{DefaultFormatter, FemtoHandlerTrait, FemtoStreamHandler};
    use std::sync::{Arc, Mutex};
    let buf = Arc::new(Mutex::new(Vec::new()));
    let handler = Arc::new(FemtoStreamHandler::new(
        SharedBuf(Arc::clone(&buf)),
        DefaultFormatter,
    ));

    let mut l1 = FemtoLogger::new("one".to_string());
    let mut l2 = FemtoLogger::new("two".to_string());
    let handler_trait: Arc<dyn FemtoHandlerTrait> = handler.clone();
    l1.add_handler(handler_trait.clone());
    l2.add_handler(handler_trait);

    l1.log("INFO", "first");
    l2.log("INFO", "second");
    drop(l1);
    drop(l2);
    drop(handler);

    let output = read_output(&buf);
    assert!(output.contains("one [INFO] first"));
    assert!(output.contains("two [INFO] second"));
    assert_eq!(output.lines().count(), 2);
}
