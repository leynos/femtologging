//! Tests logger output and level updates while a producer is active.

use std::{
    collections::BTreeSet,
    sync::{Arc, Barrier},
    thread,
};

use _femtologging_rs::{FemtoHandlerTrait, FemtoLevel, FemtoLogger};
use rstest::rstest;

use super::{ALL_LEVELS, HandlerTuple, handler_tuple, read_output};

const RACE_RECORD_COUNT: usize = 1000;

#[rstest]
fn logging_during_level_change(#[from(handler_tuple)] (buffer, handler): HandlerTuple) {
    let shared_handler: Arc<dyn FemtoHandlerTrait> = Arc::new(handler);
    let logger = Arc::new(FemtoLogger::new("race".to_owned()));
    logger.add_handler(Arc::clone(&shared_handler));
    let barrier = Arc::new(Barrier::new(2));

    let (logger_clone, barrier_clone) = (Arc::clone(&logger), Arc::clone(&barrier));
    let producer = thread::spawn(move || {
        barrier_clone.wait();
        (0..RACE_RECORD_COUNT)
            .filter_map(|index| {
                let message = format!("msg{index}");
                logger_clone
                    .log(FemtoLevel::Info, &message)
                    .is_some()
                    .then_some(message)
            })
            .collect::<BTreeSet<_>>()
    });
    barrier.wait();
    for level in ALL_LEVELS.iter().cycle().take(RACE_RECORD_COUNT) {
        logger.set_level(*level);
    }

    let accepted_messages = producer.join().expect("producer thread panicked");
    assert!(
        accepted_messages.len() <= RACE_RECORD_COUNT,
        "accepted record count ({}) must not exceed {RACE_RECORD_COUNT}",
        accepted_messages.len(),
    );
    let expected_final = ALL_LEVELS
        .iter()
        .copied()
        .cycle()
        .nth(RACE_RECORD_COUNT - 1)
        .expect("the level sequence must contain an entry");
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

    drop((logger, shared_handler));

    let output = read_output(&buffer).expect("handler output should be valid UTF-8");
    let actual_messages = output
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
