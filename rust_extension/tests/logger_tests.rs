//! Behavioural tests for `FemtoLogger`: message formatting, level filtering,
//! handler attachment and removal, and the thread-safety of both.

use std::collections::BTreeSet;

use _femtologging_rs::FemtoLogger;
use _femtologging_rs::{DefaultFormatter, FemtoHandlerTrait, FemtoLevel, FemtoStreamHandler};
use rstest::{fixture, rstest};

#[path = "test_utils/fixtures.rs"]
mod fixtures;
#[path = "test_utils/shared_buffer.rs"]
mod shared_buffer;
use fixtures::{handler_tuple, stream_handler_for};
use shared_buffer::std::{SharedBuf, read_output};
use std::sync::{Arc, Mutex};

/// A shared in-memory buffer paired with the handler writing into it.
type HandlerTuple = (Arc<Mutex<Vec<u8>>>, FemtoStreamHandler);
/// Two buffers, their native handlers, and the logger that dispatches to them.
type DualHandlerSetup = (
    Arc<Mutex<Vec<u8>>>,
    Arc<Mutex<Vec<u8>>>,
    Arc<dyn FemtoHandlerTrait>,
    Arc<dyn FemtoHandlerTrait>,
    FemtoLogger,
);

fn attach_native_handler(logger: &FemtoLogger, handler: Arc<dyn FemtoHandlerTrait>) {
    logger
        .add_handler(handler)
        .expect("native test handler should register");
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
    let logger = FemtoLogger::new("core".to_string()).expect("test logger should be constructible");
    (buf1, buf2, handler1, handler2, logger)
}

#[rstest]
#[case("core", FemtoLevel::Info, "hello", "core [INFO] hello")]
#[case("sys", FemtoLevel::Error, "fail", "sys [ERROR] fail")]
#[case("core", FemtoLevel::Info, "", "core [INFO] ")]
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
    let logger = FemtoLogger::new(name.to_string()).expect("test logger should be constructible");
    assert_eq!(logger.log(level, message).as_deref(), Some(expected));
}

/// A logger name is a component identifier, and an empty one is not.
///
/// This case used to live in `log_formats_message` as a logger named `""`
/// with an empty message, which conflated two things: an empty *message* is
/// ordinary, an empty *name* is refused by validation. The message case is
/// still covered above; the name case asserts the refusal, and asserts it here
/// rather than only in the manager's unit tests so the constructor's public
/// contract is what is being pinned.
#[test]
fn an_empty_logger_name_is_refused() {
    // `let ... else` rather than `expect_err`: the latter needs the success
    // type to be `Debug`, and `FemtoLogger` deliberately is not.
    let Err(error) = FemtoLogger::new(String::new()) else {
        panic!("an empty logger name must be refused");
    };
    assert!(
        error.to_string().contains("empty"),
        "the diagnostic should say the name is empty, got {error}"
    );
}

#[rstest]
#[case(0)]
#[case(1024)]
#[case(65536)]
#[case(1_048_576)]
fn log_formats_long_messages(#[case] length: usize) {
    let msg = "x".repeat(length);
    let logger = FemtoLogger::new("long".to_string()).expect("test logger should be constructible");
    let expected = format!("long [INFO] {}", msg);
    assert_eq!(
        logger.log(FemtoLevel::Info, &msg).as_deref(),
        Some(expected.as_str())
    );
}

#[test]
fn logger_filters_levels() {
    let logger = FemtoLogger::new("core".to_string()).expect("test logger should be constructible");
    logger.set_level(FemtoLevel::Error);
    assert_eq!(logger.log(FemtoLevel::Info, "ignored"), None);
    assert_eq!(
        logger.log(FemtoLevel::Error, "processed").as_deref(),
        Some("core [ERROR] processed")
    );
}

#[test]
fn level_parsing_and_filtering() {
    let logger = FemtoLogger::new("core".to_string()).expect("test logger should be constructible");
    for lvl in ALL_LEVELS {
        logger.set_level(lvl);
        assert!(logger.log(lvl, "ok").is_some());
    }

    logger.set_level(FemtoLevel::Error);
    assert!(logger.log(FemtoLevel::Warn, "drop").is_none());
}

#[rstest]
fn logger_routes_to_multiple_handlers(
    #[from(dual_handler_setup)] (buf1, buf2, handler1, handler2, logger): DualHandlerSetup,
) {
    attach_native_handler(&logger, handler1.clone());
    attach_native_handler(&logger, handler2.clone());
    logger.log(FemtoLevel::Info, "hello");
    drop(logger);
    drop(handler1);
    drop(handler2);
    assert_eq!(read_output(&buf1), "core [INFO] hello\n");
    assert_eq!(read_output(&buf2), "core [INFO] hello\n");
}

#[rstest]
fn shared_handler_across_loggers(#[from(handler_tuple)] (buffer, handler): HandlerTuple) {
    let handler = Arc::new(handler);
    let l1 = FemtoLogger::new("a".to_string()).expect("test logger should be constructible");
    let l2 = FemtoLogger::new("b".to_string()).expect("test logger should be constructible");
    attach_native_handler(&l1, handler.clone() as Arc<dyn FemtoHandlerTrait>);
    attach_native_handler(&l2, handler.clone() as Arc<dyn FemtoHandlerTrait>);
    l1.log(FemtoLevel::Info, "one");
    l2.log(FemtoLevel::Info, "two");
    drop(l1);
    drop(l2);
    drop(handler);
    let out = read_output(&buffer);
    assert!(out.contains("a [INFO] one"));
    assert!(out.contains("b [INFO] two"));
}

#[rstest]
fn adding_same_handler_multiple_times_duplicates_output(
    #[from(handler_tuple)] (buffer, handler): HandlerTuple,
) {
    let handler: Arc<dyn FemtoHandlerTrait> = Arc::new(handler);
    let logger = FemtoLogger::new("dup".to_string()).expect("test logger should be constructible");
    attach_native_handler(&logger, handler.clone());
    attach_native_handler(&logger, handler.clone());
    logger.log(FemtoLevel::Info, "hello");
    drop(logger);
    drop(handler);
    assert_eq!(read_output(&buffer), "dup [INFO] hello\ndup [INFO] hello\n");
}

#[rstest]
fn handler_added_after_logging_only_sees_future_records(
    #[from(dual_handler_setup)] (buf1, buf2, h1, h2, logger): DualHandlerSetup,
) {
    attach_native_handler(&logger, h1.clone());
    logger.log(FemtoLevel::Info, "before");
    attach_native_handler(&logger, h2.clone());
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
#[rstest]
fn handler_can_be_removed(#[from(handler_tuple)] (buffer, handler): HandlerTuple) {
    let handler: Arc<dyn FemtoHandlerTrait> = Arc::new(handler);
    let logger = FemtoLogger::new("core".to_string()).expect("test logger should be constructible");
    attach_native_handler(&logger, Arc::clone(&handler));
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

#[rstest]
fn logger_drains_records_on_drop(#[from(handler_tuple)] (buffer, handler): HandlerTuple) {
    let handler = Arc::new(handler);
    let logger = FemtoLogger::new("core".to_string()).expect("test logger should be constructible");
    attach_native_handler(&logger, handler.clone() as Arc<dyn FemtoHandlerTrait>);
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

#[test]
fn add_handler_is_thread_safe() {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let logger = Arc::new(
        FemtoLogger::new("core".to_string()).expect("test logger should be constructible"),
    );
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
                attach_native_handler(&log_clone, h);
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
    let output = read_output(&buffer);
    assert_eq!(output.lines().count(), 4);
}

#[test]
fn get_level_returns_current_level() {
    let logger = FemtoLogger::new("core".to_string()).expect("test logger should be constructible");
    for lvl in ALL_LEVELS {
        logger.set_level(lvl);
        assert_eq!(logger.get_level(), lvl);
    }
}

#[test]
fn set_level_is_thread_safe() {
    use std::sync::Barrier;
    use std::thread;

    let logger = Arc::new(
        FemtoLogger::new("concurrent".to_string()).expect("test logger should be constructible"),
    );
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

const RACE_RECORD_COUNT: usize = 1000;

#[rstest]
fn logging_during_level_change(#[from(handler_tuple)] (buffer, handler): HandlerTuple) {
    use std::sync::Barrier;
    use std::thread;

    let handler: Arc<dyn FemtoHandlerTrait> = Arc::new(handler);
    let logger = Arc::new(
        FemtoLogger::new("race".to_string()).expect("test logger should be constructible"),
    );
    attach_native_handler(&logger, Arc::clone(&handler));
    let barrier = Arc::new(Barrier::new(2));

    let (lg, b) = (Arc::clone(&logger), Arc::clone(&barrier));
    let producer = thread::spawn(move || {
        b.wait();
        (0..RACE_RECORD_COUNT)
            .filter_map(|index| {
                let message = format!("msg{index}");
                lg.log(FemtoLevel::Info, &message)
                    .is_some()
                    .then_some(message)
            })
            .collect::<BTreeSet<_>>()
    });
    barrier.wait();
    for lvl in ALL_LEVELS.iter().cycle().take(RACE_RECORD_COUNT) {
        logger.set_level(*lvl);
    }

    let accepted_messages = producer.join().expect("producer thread panicked");
    assert!(
        accepted_messages.len() <= RACE_RECORD_COUNT,
        "accepted record count ({}) must not exceed {RACE_RECORD_COUNT}",
        accepted_messages.len(),
    );
    let expected_final = ALL_LEVELS[(RACE_RECORD_COUNT - 1) % ALL_LEVELS.len()];
    assert_eq!(
        logger.get_level(),
        expected_final,
        "the final set_level must be observable after the race",
    );
    logger.set_level(FemtoLevel::Trace);
    assert!(
        logger.log(FemtoLevel::Info, "after").is_some(),
        "logger should remain usable after the race",
    );
    logger.set_level(FemtoLevel::Critical);
    assert!(
        logger.log(FemtoLevel::Info, "suppressed").is_none(),
        "logger should still suppress records below its level after the race",
    );

    drop((logger, handler));

    let actual_messages = read_output(&buffer)
        .lines()
        .map(|line| {
            line.strip_prefix("race [INFO] ")
                .expect("race handler output should use the default formatter")
                .to_owned()
        })
        .collect::<Vec<_>>();
    let actual_set = actual_messages.iter().cloned().collect::<BTreeSet<_>>();
    let mut expected_messages = accepted_messages;
    expected_messages.insert("after".to_owned());
    assert_eq!(
        actual_messages.len(),
        actual_set.len(),
        "handler output should not duplicate identities: {actual_messages:?}",
    );
    assert_eq!(
        actual_set, expected_messages,
        "output identities should match accepted records"
    );
}
