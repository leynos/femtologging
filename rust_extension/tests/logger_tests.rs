use _femtologging_rs::FemtoLogger;
use _femtologging_rs::QueuedRecord; // needed for clone_sender test
use _femtologging_rs::{
    DefaultFormatter, FemtoHandlerTrait, FemtoLevel, FemtoLogRecord, FemtoStreamHandler,
};
use rstest::{fixture, rstest};
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

#[fixture]
fn dual_handler_setup() -> (
    Arc<Mutex<Vec<u8>>>,
    Arc<Mutex<Vec<u8>>>,
    Arc<dyn FemtoHandlerTrait>,
    Arc<dyn FemtoHandlerTrait>,
    FemtoLogger,
) {
    let buf1 = Arc::new(Mutex::new(Vec::new()));
    let buf2 = Arc::new(Mutex::new(Vec::new()));
    let handler1: Arc<dyn FemtoHandlerTrait> = Arc::new(FemtoStreamHandler::new(
        SharedBuf(Arc::clone(&buf1)),
        DefaultFormatter,
    ));
    let handler2: Arc<dyn FemtoHandlerTrait> = Arc::new(FemtoStreamHandler::new(
        SharedBuf(Arc::clone(&buf2)),
        DefaultFormatter,
    ));
    let logger = FemtoLogger::new("core".to_string());
    (buf1, buf2, handler1, handler2, logger)
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

#[rstest]
fn logger_routes_to_multiple_handlers(
    #[from(dual_handler_setup)] (buf1, buf2, handler1, handler2, mut logger): (
        Arc<Mutex<Vec<u8>>>,
        Arc<Mutex<Vec<u8>>>,
        Arc<dyn FemtoHandlerTrait>,
        Arc<dyn FemtoHandlerTrait>,
        FemtoLogger,
    ),
) {
    logger.add_handler(handler1.clone());
    logger.add_handler(handler2.clone());
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
    let l1 = FemtoLogger::new("a".to_string());
    let l2 = FemtoLogger::new("b".to_string());
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
fn adding_same_handler_multiple_times_duplicates_output() {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let handler: Arc<dyn FemtoHandlerTrait> = Arc::new(FemtoStreamHandler::new(
        SharedBuf(Arc::clone(&buffer)),
        DefaultFormatter,
    ));
    let logger = FemtoLogger::new("dup".to_string());
    logger.add_handler(handler.clone());
    logger.add_handler(handler.clone());
    logger.log(FemtoLevel::Info, "hello");
    drop(logger);
    drop(handler);
    assert_eq!(read_output(&buffer), "dup [INFO] hello\ndup [INFO] hello\n");
}

#[rstest]
fn handler_added_after_logging_only_sees_future_records(
    #[from(dual_handler_setup)] (buf1, buf2, h1, h2, mut logger): (
        Arc<Mutex<Vec<u8>>>,
        Arc<Mutex<Vec<u8>>>,
        Arc<dyn FemtoHandlerTrait>,
        Arc<dyn FemtoHandlerTrait>,
        FemtoLogger,
    ),
) {
    logger.add_handler(h1.clone());
    logger.log(FemtoLevel::Info, "before");
    logger.add_handler(h2.clone());
    logger.log(FemtoLevel::Info, "after");
    drop(logger);
    drop(h1);
    drop(h2);
    assert_eq!(
        read_output(&buf1),
        "core [INFO] before\ncore [INFO] after\n"
    );
    assert_eq!(read_output(&buf2), "core [INFO] after\n");
}
#[test]
fn handler_can_be_removed() {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let handler: Arc<dyn FemtoHandlerTrait> = Arc::new(FemtoStreamHandler::new(
        SharedBuf(Arc::clone(&buffer)),
        DefaultFormatter,
    ));
    let logger = FemtoLogger::new("core".to_string());
    logger.add_handler(Arc::clone(&handler));
    logger.log(FemtoLevel::Info, "one");
    handler.flush();
    std::thread::sleep(std::time::Duration::from_millis(10));
    let output = read_output(&buffer);
    assert!(output.contains("one"));
    assert!(logger.remove_handler(&handler));
    logger.log(FemtoLevel::Info, "two");
    drop(logger);
    handler.flush();
    drop(handler);
    let output = read_output(&buffer);
    assert!(!output.contains("two"));
}

#[test]
fn drop_with_sender_clone_exits() {
    let logger = FemtoLogger::new("clone".to_string());
    let tx = logger.clone_sender_for_test().expect("sender should exist");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let thread_barrier = std::sync::Arc::clone(&barrier);
    let t = std::thread::spawn(move || {
        thread_barrier.wait();
        let res = tx.send(QueuedRecord {
            record: FemtoLogRecord::new("clone", "INFO", "late"),
            handlers: Vec::new(),
        });
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
    let logger = FemtoLogger::new("core".to_string());
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
