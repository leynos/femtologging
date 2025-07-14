use _femtologging_rs::FemtoLogger;
use _femtologging_rs::{
    DefaultFormatter, FemtoHandlerTrait, FemtoLevel, FemtoLogRecord, FemtoStreamHandler,
};
use rstest::rstest;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct SharedBuf(Arc<Mutex<Vec<u8>>>);

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("Failed to lock SharedBuf for writing")
            .write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0
            .lock()
            .expect("Failed to lock SharedBuf for flushing")
            .flush()
    }
}

fn read_output(buffer: &Arc<Mutex<Vec<u8>>>) -> String {
    String::from_utf8(
        buffer
            .lock()
            .expect("Failed to lock buffer for reading")
            .clone(),
    )
    .expect("Buffer did not contain valid UTF-8")
}

#[rstest]
#[case("core", FemtoLevel::Info, "hello", "core [INFO] hello")]
#[case("sys", FemtoLevel::Error, "fail", "sys [ERROR] fail")]
#[case("", FemtoLevel::Info, "", " [INFO] ")]
#[case("core", FemtoLevel::Warn, "⚠", "core [WARN] ⚠")]
#[case(
    "i18n",
    FemtoLevel::Info,
    "こんにちは世界",
    "i18n [INFO] こんにちは世界"
)]
fn log_formats_message(
    #[case] name: &str,
    #[case] level: FemtoLevel,
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
    assert_eq!(
        logger.log(FemtoLevel::Info, &msg).as_deref(),
        Some(expected.as_str())
    );
}

#[test]
fn logger_filters_levels() {
    let logger = FemtoLogger::new("core".to_string());
    logger.set_level(FemtoLevel::Error);
    assert_eq!(logger.log(FemtoLevel::Info, "ignored"), None);
    assert_eq!(
        logger.log(FemtoLevel::Error, "processed").as_deref(),
        Some("core [ERROR] processed")
    );
}

#[test]
fn level_parsing_and_filtering() {
    let logger = FemtoLogger::new("core".to_string());
    for lvl in [
        FemtoLevel::Trace,
        FemtoLevel::Debug,
        FemtoLevel::Info,
        FemtoLevel::Warn,
        FemtoLevel::Error,
        FemtoLevel::Critical,
    ] {
        logger.set_level(lvl);
        assert!(logger.log(lvl, "ok").is_some());
    }

    logger.set_level(FemtoLevel::Error);
    assert!(logger.log(FemtoLevel::Warn, "drop").is_none());
}

#[test]
fn logger_routes_to_multiple_handlers() {
    let buf1 = Arc::new(Mutex::new(Vec::new()));
    let buf2 = Arc::new(Mutex::new(Vec::new()));
    let handler1 = Arc::new(FemtoStreamHandler::new(
        SharedBuf(Arc::clone(&buf1)),
        DefaultFormatter,
    ));
    let handler2 = Arc::new(FemtoStreamHandler::new(
        SharedBuf(Arc::clone(&buf2)),
        DefaultFormatter,
    ));
    let mut logger = FemtoLogger::new("core".to_string());
    logger.add_handler(handler1.clone() as Arc<dyn FemtoHandlerTrait>);
    logger.add_handler(handler2.clone() as Arc<dyn FemtoHandlerTrait>);
    logger.log(FemtoLevel::Info, "hello");
    drop(logger);
    drop(handler1);
    drop(handler2);
    assert_eq!(read_output(&buf1), "core [INFO] hello\n");
    assert_eq!(read_output(&buf2), "core [INFO] hello\n");
}

#[test]
fn shared_handler_across_loggers() {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let handler = Arc::new(FemtoStreamHandler::new(
        SharedBuf(Arc::clone(&buffer)),
        DefaultFormatter,
    ));
    let mut l1 = FemtoLogger::new("a".to_string());
    let mut l2 = FemtoLogger::new("b".to_string());
    l1.add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>);
    l2.add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>);
    l1.log(FemtoLevel::Info, "one");
    l2.log(FemtoLevel::Info, "two");
    drop(l1);
    drop(l2);
    drop(handler);
    let out = read_output(&buffer);
    assert!(out.contains("a [INFO] one"));
    assert!(out.contains("b [INFO] two"));
}

#[test]
fn drop_with_sender_clone_exits() {
    let logger = FemtoLogger::new("clone".to_string());
    let tx = logger.clone_sender_for_test().expect("sender should exist");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let thread_barrier = std::sync::Arc::clone(&barrier);
    let t = std::thread::spawn(move || {
        thread_barrier.wait();
        let res = tx.send(FemtoLogRecord::new("clone", "INFO", "late"));
        assert!(
            res.is_err(),
            "Expected send to fail after logger is dropped"
        );
    });
    drop(logger);
    barrier.wait();
    t.join().expect("Worker thread panicked");
}

#[test]
fn logger_drains_records_on_drop() {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let handler = Arc::new(FemtoStreamHandler::new(
        SharedBuf(Arc::clone(&buffer)),
        DefaultFormatter,
    ));
    let mut logger = FemtoLogger::new("core".to_string());
    logger.add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>);
    logger.log(FemtoLevel::Info, "one");
    logger.log(FemtoLevel::Info, "two");
    logger.log(FemtoLevel::Info, "three");
    drop(logger);
    drop(handler);
    assert_eq!(
        read_output(&buffer),
        "core [INFO] one\ncore [INFO] two\ncore [INFO] three\n"
    );
}
