//! Integration coverage for production log-facade maximum-level updates.

#![cfg(feature = "log-compat")]

use _femtologging_rs::{FemtoLevel, get_logger_for_log_compat, install_global_logger};
use proptest::prelude::*;
use proptest::test_runner::{Config as ProptestConfig, TestRunner};
use pyo3::Python;
use std::cell::Cell;

/// Number of loggers updated by generated operations, including root.
const LOGGER_COUNT: usize = 5;
/// Explicit oracle floor matching the documented default for new loggers.
const DEFAULT_LEVEL: FemtoLevel = FemtoLevel::Info;
/// Concrete femtologging thresholds sampled by generated operations.
const GENERATED_LEVELS: [FemtoLevel; 6] = [
    FemtoLevel::Trace,
    FemtoLevel::Debug,
    FemtoLevel::Info,
    FemtoLevel::Warn,
    FemtoLevel::Error,
    FemtoLevel::Critical,
];

/// Independently compute the most permissive level accepted by any logger.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(expected_maximum([FemtoLevel::Warn].into_iter()), log::LevelFilter::Info);
/// ```
fn expected_maximum(levels: impl Iterator<Item = FemtoLevel>) -> log::LevelFilter {
    levels
        .chain(std::iter::once(DEFAULT_LEVEL))
        .map(|level| match level {
            FemtoLevel::Trace => log::LevelFilter::Trace,
            FemtoLevel::Debug => log::LevelFilter::Debug,
            FemtoLevel::Info => log::LevelFilter::Info,
            FemtoLevel::Warn => log::LevelFilter::Warn,
            FemtoLevel::Error => log::LevelFilter::Error,
            FemtoLevel::Critical => log::LevelFilter::Off,
        })
        .max()
        .expect("the default logger level is always included")
}

/// Check facade maxima after real setter calls over generated update sequences.
#[test]
fn published_maximum_tracks_production_logger_update_sequences() {
    Python::attach(|py| {
        let mut loggers = Vec::with_capacity(LOGGER_COUNT);
        loggers.push(get_logger_for_log_compat(py, "root").expect("root logger should register"));
        for index in 1..LOGGER_COUNT {
            let name = format!("bridge.production_maximum.{index}");
            loggers.push(get_logger_for_log_compat(py, &name).expect("logger should register"));
        }

        assert!(
            install_global_logger(),
            "the integration test owns log setup"
        );
        assert_eq!(
            log::max_level(),
            expected_maximum([DEFAULT_LEVEL; LOGGER_COUNT].into_iter()),
        );

        let updates = prop::collection::vec(
            (
                0_usize..LOGGER_COUNT,
                prop::sample::select(GENERATED_LEVELS.to_vec()),
            ),
            0..32,
        );
        let mut runner = TestRunner::new(ProptestConfig {
            cases: 48,
            ..ProptestConfig::default()
        });
        let levels: [Cell<FemtoLevel>; LOGGER_COUNT] =
            std::array::from_fn(|_| Cell::new(DEFAULT_LEVEL));

        runner
            .run(&updates, |generated_updates| {
                for (index, logger) in loggers.iter().enumerate() {
                    logger.borrow(py).set_level(DEFAULT_LEVEL);
                    levels[index].set(DEFAULT_LEVEL);
                    prop_assert_eq!(
                        log::max_level(),
                        expected_maximum(levels.iter().map(Cell::get)),
                        "maximum after resetting logger {}",
                        index,
                    );
                }

                let required_updates = [
                    (0, FemtoLevel::Debug),
                    (0, FemtoLevel::Warn),
                    (1, FemtoLevel::Critical),
                    (1, FemtoLevel::Critical),
                ];
                for (index, level) in required_updates
                    .into_iter()
                    .chain(generated_updates.iter().copied())
                {
                    loggers[index].borrow(py).set_level(level);
                    levels[index].set(level);
                    prop_assert_eq!(
                        log::max_level(),
                        expected_maximum(levels.iter().map(Cell::get)),
                        "maximum after logger {} changes to {:?}",
                        index,
                        level,
                    );
                }
                Ok(())
            })
            .expect("published maximum should match every generated update sequence");
    });
}
