//! Unit tests for the `log` crate bridge.

use log::{LevelFilter, Log};
use proptest::prelude::*;

use super::max_level::{conservative_max_level_from, map_femto_to_log_filter};
use super::*;
use crate::handler::FemtoHandlerTrait;
use crate::logger::DEFAULT_LOGGER_LEVEL;
use crate::test_utils::collecting_handler::CollectingHandler;
use rstest::{fixture, rstest};
use serial_test::serial;
use std::sync::{
    Arc, Once,
    atomic::{AtomicUsize, Ordering},
};

static LOGGER_COUNTER: AtomicUsize = AtomicUsize::new(0);

#[fixture]
fn unique_logger_name() -> String {
    let base = "bridge.test";
    let suffix = LOGGER_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{base}.{suffix}")
}

#[rstest]
#[case(log::Level::Trace, FemtoLevel::Trace)]
#[case(log::Level::Debug, FemtoLevel::Debug)]
#[case(log::Level::Info, FemtoLevel::Info)]
#[case(log::Level::Warn, FemtoLevel::Warn)]
#[case(log::Level::Error, FemtoLevel::Error)]
fn level_mapping_is_direct(#[case] level: log::Level, #[case] expected: FemtoLevel) {
    assert_eq!(FemtoLevel::from(level), expected);
}

#[rstest]
#[case::trace(FemtoLevel::Trace, log::LevelFilter::Trace)]
#[case::debug(FemtoLevel::Debug, log::LevelFilter::Debug)]
#[case::info(FemtoLevel::Info, log::LevelFilter::Info)]
#[case::warn(FemtoLevel::Warn, log::LevelFilter::Warn)]
#[case::error(FemtoLevel::Error, log::LevelFilter::Error)]
#[case::critical(FemtoLevel::Critical, log::LevelFilter::Off)]
fn femto_threshold_maps_to_its_broadest_rust_level(
    #[case] level: FemtoLevel,
    #[case] expected: log::LevelFilter,
) {
    assert_eq!(map_femto_to_log_filter(level), expected);
}

#[rstest]
fn conservative_maximum_includes_default_and_live_logger_levels() {
    assert_eq!(
        conservative_max_level_from(std::iter::empty()),
        log::LevelFilter::Info,
    );
    assert_eq!(
        conservative_max_level_from([FemtoLevel::Warn].into_iter()),
        log::LevelFilter::Info,
    );
    assert_eq!(
        conservative_max_level_from([FemtoLevel::Debug].into_iter()),
        log::LevelFilter::Debug,
    );
    assert_eq!(
        conservative_max_level_from([FemtoLevel::Critical].into_iter()),
        log::LevelFilter::Info,
    );
}

proptest! {
    /// Every threshold accepted by a target remains inside the facade maximum.
    #[test]
    fn conservative_maximum_covers_arbitrary_logger_thresholds(
        level_values in prop::collection::vec(0_u8..=u8::from(FemtoLevel::Critical), 0..32)
    ) {
        let levels = level_values
            .into_iter()
            .map(|value| FemtoLevel::try_from(value).expect("generated a valid FemtoLevel"))
            .collect::<Vec<_>>();
        let maximum = conservative_max_level_from(levels.iter().copied());
        let expected = levels
            .iter()
            .copied()
            .chain(std::iter::once(DEFAULT_LOGGER_LEVEL))
            .map(map_femto_to_log_filter)
            .max()
            .expect("the default logger level is always included");

        prop_assert_eq!(maximum, expected);
        prop_assert!(maximum >= map_femto_to_log_filter(DEFAULT_LOGGER_LEVEL));
        prop_assert!(levels.iter().all(|level| maximum >= map_femto_to_log_filter(*level)));
    }

    /// Recomputing after generated level changes stays conservative.
    #[test]
    fn conservative_maximum_tracks_logger_level_update_sequences(
        updates in prop::collection::vec(
            (0_usize..4, 0_u8..=u8::from(FemtoLevel::Critical)),
            0..48,
        )
    ) {
        let mut levels = [DEFAULT_LOGGER_LEVEL; 4];
        for (index, level_value) in updates {
            levels[index] = FemtoLevel::try_from(level_value)
                .expect("generated a valid FemtoLevel");
            let maximum = conservative_max_level_from(levels.iter().copied());
            let expected = levels
                .iter()
                .copied()
                .chain(std::iter::once(DEFAULT_LOGGER_LEVEL))
                .map(map_femto_to_log_filter)
                .max()
                .expect("the default logger level is always included");

            prop_assert_eq!(maximum, expected);
            let all_levels_are_accepted = levels
                .iter()
                .all(|level| maximum >= map_femto_to_log_filter(*level));
            prop_assert!(all_levels_are_accepted);
        }
    }
}

#[fixture]
fn log_max_level() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        log::set_max_level(LevelFilter::Trace);
    });
}

#[rstest]
#[serial]
fn adapter_dispatches_records_to_target_logger(_log_max_level: (), unique_logger_name: String) {
    let adapter = FemtoLogAdapter;
    let logger_name = unique_logger_name;

    Python::attach(|py| {
        let logger = manager::get_logger(py, &logger_name).expect("logger created");
        let handler = Arc::new(CollectingHandler::default()) as Arc<dyn FemtoHandlerTrait>;
        logger
            .borrow(py)
            .add_handler(handler.clone())
            .expect("native test handler should register");

        let record = log::Record::builder()
            .args(format_args!("hello"))
            .level(log::Level::Info)
            .target(&logger_name)
            .module_path(Some("bridge::test"))
            .file(Some("lib.rs"))
            .line(Some(42))
            .build();

        adapter.log(&record);

        assert!(
            logger.borrow(py).flush_handlers(),
            "flush should drain the queue"
        );

        let records = handler
            .as_any()
            .downcast_ref::<CollectingHandler>()
            .expect("handler downcast")
            .collected();
        assert_eq!(records.len(), 1);
        let rec = &records[0];
        assert_eq!(rec.logger(), logger_name.as_str());
        assert_eq!(rec.level_str(), "INFO");
        assert_eq!(rec.message(), "hello");
        assert_eq!(rec.metadata().module_path, "bridge::test");
        assert_eq!(rec.metadata().filename, "lib.rs");
        assert_eq!(rec.metadata().line_number, 42);
    });
}

#[rstest]
#[serial]
fn adapter_normalizes_rust_module_targets(_log_max_level: (), unique_logger_name: String) {
    let adapter = FemtoLogAdapter;
    let logger_name = unique_logger_name;
    let target = logger_name.replace('.', "::");

    Python::attach(|py| {
        let logger = manager::get_logger(py, &logger_name).expect("logger created");
        let handler = Arc::new(CollectingHandler::default()) as Arc<dyn FemtoHandlerTrait>;
        logger
            .borrow(py)
            .add_handler(handler.clone())
            .expect("native test handler should register");

        let record = log::Record::builder()
            .args(format_args!("normalized"))
            .level(log::Level::Info)
            .target(&target)
            .build();

        adapter.log(&record);
        assert!(logger.borrow(py).flush_handlers());

        let records = handler
            .as_any()
            .downcast_ref::<CollectingHandler>()
            .expect("handler downcast")
            .collected();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].logger(), logger_name.as_str());
    });
}

#[rstest]
#[serial]
fn log_respects_logger_threshold(_log_max_level: (), unique_logger_name: String) {
    let adapter = FemtoLogAdapter;
    let logger_name = unique_logger_name;

    Python::attach(|py| {
        let logger = manager::get_logger(py, &logger_name).expect("logger created");
        let handler = Arc::new(CollectingHandler::default()) as Arc<dyn FemtoHandlerTrait>;
        logger
            .borrow(py)
            .add_handler(handler.clone())
            .expect("native test handler should register");
        logger.borrow(py).set_level(FemtoLevel::Warn);

        let info_record = log::Record::builder()
            .args(format_args!("info"))
            .level(log::Level::Info)
            .target(&logger_name)
            .build();
        adapter.log(&info_record);

        let warn_record = log::Record::builder()
            .args(format_args!("warn"))
            .level(log::Level::Warn)
            .target(&logger_name)
            .build();
        adapter.log(&warn_record);

        assert!(
            logger.borrow(py).flush_handlers(),
            "flush should drain the queue"
        );

        let records = handler
            .as_any()
            .downcast_ref::<CollectingHandler>()
            .expect("handler downcast")
            .collected();
        assert_eq!(records.len(), 1, "only WARN should pass threshold");
        assert_eq!(records[0].level_str(), "WARN");
    });
}

#[rstest]
#[serial]
fn enabled_uses_the_registered_target_threshold(_log_max_level: (), unique_logger_name: String) {
    let adapter = FemtoLogAdapter;

    Python::attach(|py| {
        let logger = manager::get_logger(py, &unique_logger_name).expect("logger created");
        logger.borrow(py).set_level(FemtoLevel::Warn);
    });

    for (level, expected) in [
        (log::Level::Trace, false),
        (log::Level::Debug, false),
        (log::Level::Info, false),
        (log::Level::Warn, true),
        (log::Level::Error, true),
    ] {
        let metadata = Metadata::builder()
            .level(level)
            .target(&unique_logger_name)
            .build();
        assert_eq!(adapter.enabled(&metadata), expected, "level {level}");
    }
}

#[rstest]
#[serial]
fn enabled_normalizes_module_targets_and_leaves_unknown_targets_unregistered(
    _log_max_level: (),
    unique_logger_name: String,
) {
    let adapter = FemtoLogAdapter;
    let rust_target = unique_logger_name.replace('.', "::");

    Python::attach(|py| {
        let logger = manager::get_logger(py, &unique_logger_name).expect("logger created");
        logger.borrow(py).set_level(FemtoLevel::Warn);
    });

    let normalized_metadata = Metadata::builder()
        .level(log::Level::Debug)
        .target(&rust_target)
        .build();
    assert!(!adapter.enabled(&normalized_metadata));

    let unknown_target = format!("{unique_logger_name}.missing");
    assert_eq!(manager::registered_logger_level(&unknown_target), None);
    let unknown_metadata = Metadata::builder()
        .level(log::Level::Info)
        .target(&unknown_target)
        .build();
    assert!(adapter.enabled(&unknown_metadata));
    assert_eq!(manager::registered_logger_level(&unknown_target), None);
}

#[rstest]
#[serial]
fn enabled_uses_root_for_invalid_targets_and_default_when_root_is_absent(_log_max_level: ()) {
    let adapter = FemtoLogAdapter;
    manager::reset_manager();

    let invalid_info = Metadata::builder()
        .level(log::Level::Info)
        .target("invalid..target")
        .build();
    assert!(adapter.enabled(&invalid_info));
    assert_eq!(manager::registered_logger_level("root"), None);

    Python::attach(|py| {
        let root = manager::get_logger(py, "root").expect("root logger created");
        root.borrow(py).set_level(FemtoLevel::Warn);
    });

    let invalid_debug = Metadata::builder()
        .level(log::Level::Debug)
        .target("invalid..target")
        .build();
    let invalid_warn = Metadata::builder()
        .level(log::Level::Warn)
        .target("invalid..target")
        .build();
    assert!(!adapter.enabled(&invalid_debug));
    assert!(!adapter.enabled(&invalid_info));
    assert!(adapter.enabled(&invalid_warn));
    assert_eq!(
        manager::registered_logger_level("root"),
        Some(FemtoLevel::Warn)
    );
}
