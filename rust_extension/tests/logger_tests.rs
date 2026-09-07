//! Behavioural tests for `FemtoLogger`: message formatting, level filtering,
//! handler attachment and removal, and the thread-safety of both.

use _femtologging_rs::{
    DefaultFormatter,
    FemtoHandlerTrait,
    FemtoLevel,
    FemtoLogRecord,
    FemtoStreamHandler,
};
use _femtologging_rs::{FemtoLogger, QueuedRecord}; // needed for clone_sender test
use rstest::{fixture, rstest};

#[path = "test_utils/fixtures.rs"]
mod fixtures;
#[path = "test_utils/shared_buffer.rs"]
mod shared_buffer;
use std::sync::{Arc, Mutex};

use fixtures::{handler_tuple, stream_handler_for};
use shared_buffer::std::{SharedBuf, read_output};

/// A shared in-memory buffer paired with the handler writing into it.
type HandlerTuple = (Arc<Mutex<Vec<u8>>>, FemtoStreamHandler);

/// The two buffers and handlers used by tests that exercise fan-out.
struct DualHandlerSetup {
    buf1: Arc<Mutex<Vec<u8>>>,
    buf2: Arc<Mutex<Vec<u8>>>,
    handler1: Arc<dyn FemtoHandlerTrait>,
    handler2: Arc<dyn FemtoHandlerTrait>,
    logger: FemtoLogger,
}

/// Every level a logger may be set to, ordered from most to least verbose.
const ALL_LEVELS: [FemtoLevel; 6] = [
    FemtoLevel::Trace,
    FemtoLevel::Debug,
    FemtoLevel::Info,
    FemtoLevel::Warn,
    FemtoLevel::Error,
    FemtoLevel::Critical,
];

#[fixture]
fn dual_handler_setup() -> DualHandlerSetup {
    let buf1 = Arc::new(Mutex::new(Vec::new()));
    let buf2 = Arc::new(Mutex::new(Vec::new()));
    let handler1: Arc<dyn FemtoHandlerTrait> = Arc::new(FemtoStreamHandler::new(
        SharedBuf::new(Arc::clone(&buf1)),
        DefaultFormatter,
    ));
    let handler2: Arc<dyn FemtoHandlerTrait> = Arc::new(FemtoStreamHandler::new(
        SharedBuf::new(Arc::clone(&buf2)),
        DefaultFormatter,
    ));
    let logger = FemtoLogger::new("core".to_owned());
    DualHandlerSetup {
        buf1,
        buf2,
        handler1,
        handler2,
        logger,
    }
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
    let logger = FemtoLogger::new(name.to_owned());
    assert_eq!(logger.log(level, message).as_deref(), Some(expected));
}

#[rstest]
#[case(0)]
#[case(1024)]
#[case(65536)]
#[case(1_048_576)]
fn log_formats_long_messages(#[case] length: usize) {
    let msg = "x".repeat(length);
    let logger = FemtoLogger::new("long".to_owned());
    let expected = format!("long [INFO] {msg}");
    assert_eq!(
        logger.log(FemtoLevel::Info, &msg).as_deref(),
        Some(expected.as_str())
    );
}

#[test]
fn logger_filters_levels() {
    let logger = FemtoLogger::new("core".to_owned());
    logger.set_level(FemtoLevel::Error);
    assert_eq!(logger.log(FemtoLevel::Info, "ignored"), None);
    assert_eq!(
        logger.log(FemtoLevel::Error, "processed").as_deref(),
        Some("core [ERROR] processed")
    );
}

#[test]
fn level_parsing_and_filtering() {
    let logger = FemtoLogger::new("core".to_owned());
    for lvl in ALL_LEVELS {
        logger.set_level(lvl);
        assert!(logger.log(lvl, "ok").is_some());
    }

    logger.set_level(FemtoLevel::Error);
    assert!(logger.log(FemtoLevel::Warn, "drop").is_none());
}

#[rstest]
fn logger_routes_to_multiple_handlers(#[from(dual_handler_setup)] setup: DualHandlerSetup) {
    let DualHandlerSetup {
        buf1,
        buf2,
        handler1,
        handler2,
        logger,
    } = setup;
    logger.add_handler(handler1.clone());
    logger.add_handler(handler2.clone());
    logger.log(FemtoLevel::Info, "hello");
    drop(logger);
    drop(handler1);
    drop(handler2);
    let output1 = read_output(&buf1).expect("handler output should be valid UTF-8");
    let output2 = read_output(&buf2).expect("handler output should be valid UTF-8");
    assert_eq!(output1, "core [INFO] hello\n");
    assert_eq!(output2, "core [INFO] hello\n");
}

#[rstest]
fn shared_handler_across_loggers(#[from(handler_tuple)] (buffer, handler): HandlerTuple) {
    let shared_handler = Arc::new(handler);
    let l1 = FemtoLogger::new("a".to_owned());
    let l2 = FemtoLogger::new("b".to_owned());
    l1.add_handler(shared_handler.clone() as Arc<dyn FemtoHandlerTrait>);
    l2.add_handler(shared_handler.clone() as Arc<dyn FemtoHandlerTrait>);
    l1.log(FemtoLevel::Info, "one");
    l2.log(FemtoLevel::Info, "two");
    drop(l1);
    drop(l2);
    drop(shared_handler);
    let out = read_output(&buffer).expect("shared handler output should be valid UTF-8");
    assert!(out.contains("a [INFO] one"));
    assert!(out.contains("b [INFO] two"));
}

#[rstest]
fn adding_same_handler_multiple_times_duplicates_output(
    #[from(handler_tuple)] (buffer, handler): HandlerTuple,
) {
    let shared_handler: Arc<dyn FemtoHandlerTrait> = Arc::new(handler);
    let logger = FemtoLogger::new("dup".to_owned());
    logger.add_handler(shared_handler.clone());
    logger.add_handler(shared_handler.clone());
    logger.log(FemtoLevel::Info, "hello");
    drop(logger);
    drop(shared_handler);
    let output = read_output(&buffer).expect("handler output should be valid UTF-8");
    assert_eq!(output, "dup [INFO] hello\ndup [INFO] hello\n");
}

#[rstest]
fn handler_added_after_logging_only_sees_future_records(
    #[from(dual_handler_setup)] setup: DualHandlerSetup,
) {
    let DualHandlerSetup {
        buf1,
        buf2,
        handler1: h1,
        handler2: h2,
        logger,
    } = setup;
    logger.add_handler(h1.clone());
    logger.log(FemtoLevel::Info, "before");
    logger.add_handler(h2.clone());
    logger.log(FemtoLevel::Info, "after");
    drop(logger);
    drop(h1);
    drop(h2);
    let output1 = read_output(&buf1).expect("handler output should be valid UTF-8");
    let output2 = read_output(&buf2).expect("handler output should be valid UTF-8");
    assert_eq!(output1, "core [INFO] before\ncore [INFO] after\n");
    assert_eq!(output2, "core [INFO] after\n");
}
#[rstest]
fn handler_can_be_removed(#[from(handler_tuple)] (buffer, handler): HandlerTuple) {
    let shared_handler: Arc<dyn FemtoHandlerTrait> = Arc::new(handler);
    let logger = FemtoLogger::new("core".to_owned());
    logger.add_handler(Arc::clone(&shared_handler));
    logger.log(FemtoLevel::Info, "one");
    shared_handler.flush();
    std::thread::sleep(std::time::Duration::from_millis(10));
    let output = read_output(&buffer).expect("valid UTF-8");
    assert!(output.contains("one"));
    assert!(logger.remove_handler(&shared_handler));
    logger.log(FemtoLevel::Info, "two");
    drop(logger);
    shared_handler.flush();
    drop(shared_handler);
    let final_output = read_output(&buffer).expect("handler output should be valid UTF-8");
    assert!(!final_output.contains("two"));
}

#[test]
fn drop_with_sender_clone_exits() {
    let logger = FemtoLogger::new("clone".to_owned());
    let tx = logger.clone_sender_for_test().expect("sender should exist");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let thread_barrier = std::sync::Arc::clone(&barrier);
    let t = std::thread::spawn(move || {
        thread_barrier.wait();
        let res = tx.send(QueuedRecord {
            record: FemtoLogRecord::new("clone", FemtoLevel::Info, "late"),
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

#[rstest]
fn logger_drains_records_on_drop(#[from(handler_tuple)] (buffer, handler): HandlerTuple) {
    let shared_handler = Arc::new(handler);
    let logger = FemtoLogger::new("core".to_owned());
    logger.add_handler(shared_handler.clone() as Arc<dyn FemtoHandlerTrait>);
    logger.log(FemtoLevel::Info, "one");
    logger.log(FemtoLevel::Info, "two");
    logger.log(FemtoLevel::Info, "three");
    drop(logger);
    drop(shared_handler);
    let output = read_output(&buffer).expect("handler output should be valid UTF-8");
    assert_eq!(
        output,
        "core [INFO] one\ncore [INFO] two\ncore [INFO] three\n"
    );
}

#[test]
fn add_handler_is_thread_safe() {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let logger = Arc::new(FemtoLogger::new("core".to_owned()));
    // All four handlers deliberately share one buffer so the test can count
    // the lines emitted by the whole handler set.
    let new_handlers: Vec<_> = (0..4)
        .map(|_| Arc::new(stream_handler_for(&buffer)) as Arc<dyn FemtoHandlerTrait>)
        .collect();

    let start = Arc::new(std::sync::Barrier::new(new_handlers.len() + 1));
    let worker_threads: Vec<_> = new_handlers
        .iter()
        .cloned()
        .map(|h| {
            let log_clone = Arc::clone(&logger);
            let barrier = Arc::clone(&start);
            std::thread::spawn(move || {
                barrier.wait();
                log_clone.add_handler(h);
            })
        })
        .collect();

    start.wait();
    for t in worker_threads {
        t.join().expect("thread panicked");
    }

    logger.log(FemtoLevel::Info, "hello");
    drop(logger);
    for h in new_handlers {
        drop(h);
    }
    let output = read_output(&buffer).expect("handler output should be valid UTF-8");
    assert_eq!(output.lines().count(), 4);
}

#[test]
fn get_level_returns_current_level() {
    let logger = FemtoLogger::new("core".to_owned());
    for lvl in ALL_LEVELS {
        logger.set_level(lvl);
        assert_eq!(logger.get_level(), lvl);
    }
}

#[test]
fn set_level_is_thread_safe() {
    use std::{sync::Barrier, thread};

    let logger = Arc::new(FemtoLogger::new("concurrent".to_owned()));
    let barrier = Arc::new(Barrier::new(ALL_LEVELS.len()));

    let threads: Vec<_> = ALL_LEVELS
        .into_iter()
        .map(|lvl| {
            let lg = Arc::clone(&logger);
            let b = Arc::clone(&barrier);
            thread::spawn(move || {
                b.wait();
                for _ in 0..1000 {
                    lg.set_level(lvl);
                }
            })
        })
        .collect();

    for t in threads {
        t.join().expect("thread panicked");
    }

    let final_level = logger.get_level();
    assert!(
        ALL_LEVELS.contains(&final_level),
        "Final level should be a valid FemtoLevel variant"
    );
}

#[path = "logger_cases/level_change.rs"]
mod level_change;
