//! Level-race behavioural test for `FemtoLogger`.
//!
//! A producer thread logs while the test thread cycles the logger's level.
//! The test needs its own producer result and output-identity assertion
//! helpers, so it lives beside the parent module rather than inside it.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use _femtologging_rs::{FemtoHandlerTrait, FemtoLevel, FemtoLogger};
use rstest::rstest;

use super::{
    ALL_LEVELS, HandlerTuple, attach_native_handler, fixtures::handler_tuple,
    shared_buffer::std::read_output,
};

/// Number of records the producer attempts during the race.
const RACE_RECORD_COUNT: usize = 1000;

/// Assert the level contract the logger keeps after a concurrent level race,
/// then that its handler wrote exactly the records the producer was told were
/// accepted.
///
/// The logger and handler are consumed here so that dropping them flushes the
/// queued records before the buffer is read.
#[track_caller]
fn assert_level_race_outcome(
    logger: Arc<FemtoLogger>,
    handler: Arc<dyn FemtoHandlerTrait>,
    buffer: &Arc<Mutex<Vec<u8>>>,
    accepted_messages: BTreeSet<String>,
) {
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

    let actual_messages = read_output(buffer)
        .lines()
        .map(|line| match line.strip_prefix("race [INFO] ") {
            Some(message) => message.to_owned(),
            None => panic!("race handler output should use the default formatter: {line:?}"),
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

#[rstest]
fn logging_during_level_change(#[from(handler_tuple)] (buffer, handler): HandlerTuple) {
    use std::sync::Barrier;
    use std::thread;

    let handler: Arc<dyn FemtoHandlerTrait> = Arc::new(handler);
    let logger = Arc::new(
        FemtoLogger::new("race".to_string()).expect("test logger should be constructible"),
    );
    attach_native_handler(&logger, Arc::clone(&handler))
        .expect("native test handler should register");
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
    assert_level_race_outcome(logger, handler, &buffer, accepted_messages);
}
